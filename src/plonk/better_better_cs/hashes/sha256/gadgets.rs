use crate::plonk::better_better_cs::cs::*;
use crate::plonk::better_better_cs::lookup_tables::*;
use crate::plonk::better_better_cs::utils;
use crate::pairing::ff::*;
use crate::pairing::ff::{PrimeField, PrimeFieldRepr};
use crate::SynthesisError;
use crate::Engine;
use crate::plonk::better_better_cs::gadgets::num::{
    AllocatedNum,
    Num,
};

use super::tables::*;
use crate::plonk::better_better_cs::gadgets::assignment::{
    Assignment
};

use super::utils::*;
use super::tables::*;
use super::custom_gates::*;
use std::sync::Arc;


type Result<T> = std::result::Result<T, SynthesisError>;


// helper struct for tracking how far current value from being in 32-bit range
// our gadget is suited to handle at most 4-bit overflows itself
#[derive(Copy, Clone)]
pub enum OverflowTracker {
    NoOverflow,
    OneBitOverflow,
    SmallOverflow, // overflow less or equal than 4 bits
    SignificantOverflow
}


pub struct NumWithTracker<E: Engine> {
    num: Num<E>,
    overflow_tracker: OverflowTracker,
}


#[derive(Copy, Clone)]
pub enum MajorityStrategy {
    UseTwoTables,
    RawOverflowCheck,
}


pub struct SparseChValue<E: Engine> {
    normal: Num<E>,
    sparse: Num<E>,
    // all rots are in sparse representation as well
    rot6: Num<E>,
    rot11: Num<E>,
    rot25: Num<E>,
}


pub struct SparseMajValue<E: Engine> {
    normal: Num<E>,
    sparse: Num<E>,
    // all rots are in sparse representation as well
    rot2: Num<E>,
    rot13: Num<E>,
    rot22: Num<E>,
}


pub struct Sha256GadgetParams<E: Engine> {
    // for the purpose of this flag, see comments at the beginning of "convert_into_sparse_majority_form" function
    majority_strategy: MajorityStrategy,

    // the purpose of these parameters is discussed before the "normalize" function
    ch_base_num_of_chunks: usize,
    maj_base_num_of_chunks: usize,

    // tables used for chooser (ch) implementation    
    sha256_base7_rot6_table: Arc<LookupTableApplication<E>>,
    sha256_base7_rot3_extr10_table: Arc<LookupTableApplication<E>>,
    sha256_ch_normalization_table: Arc<LookupTableApplication<E>>,
    // mod 2 normalization_table (and similar for maj - only base used will be diffent)
    sha256_ch_xor_table: Arc<LookupTableApplication<E>>,

    // tables used for majority (maj) implementation
    sha256_base4_rot2_table: Arc<LookupTableApplication<E>>,
    sha256_base4_rot2_extr10_table: Option<Arc<LookupTableApplication<E>>>,
    sha256_maj_normalization_table: Arc<LookupTableApplication<E>>,
    sha256_maj_xor_table: Arc<LookupTableApplication<E>>,

    _marker: std::marker::PhantomData<E>,
}

const SHA256_GADGET_CHUNK_SIZE : usize = 11; 
const SHA256_REG_WIDTH : usize = 32;
const CH_BASE_DEFAULT_NUM_OF_CHUNKS : usize = 4; // 7^4 is fine
const MAJ_BASE_DEFAULT_NUM_OF_CHUNKS : usize = 6; // 2^6 is fine


impl<E: Engine> Sha256GadgetParams<E> {

    pub fn new<CS: ConstraintSystem<E>>(
        cs: &mut CS, 
        majority_strategy: MajorityStrategy,
        ch_base_num_of_chunks: Option<usize>,
        maj_base_num_of_chunks: Option<usize>,
    ) -> Result<Self> {

        let ch_base_num_of_chunks = ch_base_num_of_chunks.unwrap_or(CH_BASE_DEFAULT_NUM_OF_CHUNKS);
        let maj_base_num_of_chunks = maj_base_num_of_chunks.unwrap_or(MAJ_BASE_DEFAULT_NUM_OF_CHUNKS);
        
        let columns = vec![
            PolyIdentifier::VariablesPolynomial(0), 
            PolyIdentifier::VariablesPolynomial(1), 
            PolyIdentifier::VariablesPolynomial(2)
        ];

        let name1: &'static str = "sha256_base7_rot6_table";
        let sha256_base7_rot6_table = LookupTableApplication::new(
            name1,
            Sha256SparseRotateTable::new(SHA256_GADGET_CHUNK_SIZE, 6, 0, SHA256_CHOOSE_BASE, name1),
            columns.clone(),
            true
        );

        let name2 : &'static str = "sha256_base7_rot3_extr10_table";
        let sha256_base7_rot3_extr10_table = LookupTableApplication::new(
            name2,
            Sha256SparseRotateTable::new(SHA256_GADGET_CHUNK_SIZE, 3, SHA256_GADGET_CHUNK_SIZE-1, SHA256_CHOOSE_BASE, name2),
            columns.clone(),
            true
        );

        let name3 : &'static str = "sha256_base4_rot2_table";
        let sha256_base4_rot2_table = LookupTableApplication::new(
            name3,
            Sha256SparseRotateTable::new(SHA256_GADGET_CHUNK_SIZE, 2, 0, SHA256_MAJORITY_BASE, name3),
            columns.clone(),
            true
        );

        let sha256_base7_rot6_table = cs.add_table(sha256_base7_rot6_table)?;
        let sha256_base7_rot3_extr10_table = cs.add_table(sha256_base7_rot3_extr10_table)?;
        let sha256_base4_rot2_table  = cs.add_table(sha256_base4_rot2_table)?;

        let name4 : &'static str = "sha256_base4_rot2_extr10_table";
        let sha256_base4_rot2_extr10_table = match majority_strategy {
            MajorityStrategy::RawOverflowCheck => None,
            MajorityStrategy::UseTwoTables => {
                let sha256_base4_rot2_extr10_table = LookupTableApplication::new(
                    name4,
                    Sha256SparseRotateTable::new(SHA256_GADGET_CHUNK_SIZE, 2, SHA256_GADGET_CHUNK_SIZE-1, SHA256_MAJORITY_BASE, name4),
                    columns.clone(),
                    true
                );

                Some(cs.add_table(sha256_base4_rot2_extr10_table)?)
            }
        };

        let name5 : &'static str = "sha256_ch_normalization_table";
        let sha256_ch_normalization_table = LookupTableApplication::new(
            name5,
            Sha256ChooseTable::new(ch_base_num_of_chunks, name5),
            columns.clone(),
            true
        );

        let name6 : &'static str = "sha256_maj_normalization_table";
        let sha256_maj_normalization_table = LookupTableApplication::new(
            name6,
            Sha256MajorityTable::new(maj_base_num_of_chunks, name6),
            columns.clone(),
            true
        );

        let name7 : &'static str = "sha256_ch_xor_table";
        let sha256_ch_xor_table = LookupTableApplication::new(
            name7,
            Sha256NormalizationTable::new(SHA256_CHOOSE_BASE, ch_base_num_of_chunks, name7),
            columns.clone(),
            true
        );

        let name8 : &'static str = "sha256_maj_xor_table";
        let sha256_maj_xor_table = LookupTableApplication::new(
            name8,
            Sha256NormalizationTable::new(SHA256_MAJORITY_BASE, maj_base_num_of_chunks, name8),
            columns.clone(),
            true
        );

        let sha256_ch_normalization_table = cs.add_table(sha256_ch_normalization_table)?;
        let sha256_maj_normalization_table = cs.add_table(sha256_maj_normalization_table)?;
        let sha256_ch_xor_table  = cs.add_table(sha256_ch_xor_table)?;
        let sha256_maj_xor_table  = cs.add_table(sha256_maj_xor_table)?;

        Ok(Sha256GadgetParams {
            majority_strategy,
            ch_base_num_of_chunks,
            maj_base_num_of_chunks,

            sha256_base7_rot6_table,
            sha256_base7_rot3_extr10_table,
            sha256_ch_normalization_table,
            sha256_ch_xor_table,

            sha256_base4_rot2_table,
            sha256_base4_rot2_extr10_table,
            sha256_maj_normalization_table,
            sha256_maj_xor_table,

            _marker : std::marker::PhantomData,
        })
    }

    // here we assume that maximal overflow is not more than 4 bits
    // we return both the extracted 32bit value and of_l and of_h (both - two bits long)
    fn extract_32_from_constant(x: &E::Fr) -> (E::Fr, E::Fr, E::Fr) {
        let mut repr = x.into_repr();
        let mut of_l_repr = repr.clone();
        let mut of_h_repr = repr.clone();
        
        repr.as_mut()[0] &= (1 << 32) - 1; 
        let extracted = E::Fr::from_repr(repr).expect("should decode");

        of_l_repr.as_mut()[0] >>= 32;
        of_l_repr.as_mut()[0] &= 3;
        let of_l = E::Fr::from_repr(repr).expect("should decode");

        of_h_repr.as_mut()[0] >>= 34;
        let of_h = E::Fr::from_repr(repr).expect("should decode");

        (extracted, of_l, of_h)
    } 
        
    fn extact_32_from_overflowed_num<CS: ConstraintSystem<E>>(cs: &mut CS, var: &Num<E>) -> Result<Num<E>> {
        let res = match var {
            Num::Constant(x) => {
                Num::Constant(Self::extract_32_from_constant(x).0)
            },
            Num::Allocated(x) => {
                //create a_0, a_1, ..., a_15 = extracted.
                let mut vars = Vec::with_capacity(16);
                vars.push(AllocatedNum::alloc_zero(cs)?);

                for i in 0..16 {
                    let val = x.get_value().map(| elem | {
                        let mut repr = elem.into_repr();
                        repr.as_mut()[0] >>= 30 - 2 * i;
                        let extracted = E::Fr::from_repr(repr).expect("should decode");

                        extracted
                    });

                    vars.push(AllocatedNum::alloc(cs, || val.grab())?);
                }

                for i in 0..4 {
                    let x = [vars[4*i].get_variable(), vars[4*i+1].get_variable(), vars[4*i+2].get_variable(), vars[4*i+3].get_variable()];
                    cs.new_single_gate_for_trace_step(
                        &RangeCheck32ConstraintGate::default(), 
                        &[], 
                        &x, 
                        &[]
                    )?;
                }

                let (of_l_value, of_h_value) = match x.get_value() {
                    None => (None, None),
                    Some(elem) => {
                        let temp = Self::extract_32_from_constant(&elem);
                        (Some(temp.1), Some(temp.2))
                    },
                };

                let of_l_var = AllocatedNum::alloc(cs, || of_l_value.grab())?;
                let of_h_var = AllocatedNum::alloc(cs, || of_h_value.grab())?;

                cs.begin_gates_batch_for_step()?;
                
                cs.new_gate_in_batch( 
                    &In04RangeGate::new(1),
                    &[],
                    &[x.get_variable(), of_l_var.get_variable(), of_h_var.get_variable(), vars[15].get_variable()],
                    &[],
                )?;

                cs.new_gate_in_batch( 
                    &In04RangeGate::new(2),
                    &[],
                    &[x.get_variable(), of_l_var.get_variable(), of_h_var.get_variable(), vars[15].get_variable()],
                    &[],
                )?;

                // the selectors in the main gate go in the following order:
                // [q_a, q_b, q_c, q_d, q_m, q_const, q_d_next]
                // we constraint the equation: q_a - 2^32 q_b - 2^34 q_c - q_d = 0;
                // so in our case: q_a = -1, q_b = 2^32; q_c = 2^34; q_d = 1; q_m = q_const = q_d_next = 0;

                let zero = E::Fr::zero();
                let one = E::Fr::one();
                let mut minus_one = E::Fr::one();
                minus_one.negate();

                let mut temp32_repr : <E::Fr as PrimeField>::Repr = E::Fr::zero().into_repr();
                temp32_repr.as_mut()[0] = 1 << 32;
                let coef32 = E::Fr::from_repr(temp32_repr).expect("should parse");

                let mut temp34_repr : <E::Fr as PrimeField>::Repr = E::Fr::zero().into_repr();
                temp34_repr.as_mut()[0] = 1 << 34;
                let coef34 = E::Fr::from_repr(temp34_repr).expect("should parse");

                cs.new_gate_in_batch(
                    &CS::MainGate::default(),
                    &[minus_one, coef32, coef34, one, zero.clone(), zero.clone(), zero],
                    &[x.get_variable(), of_l_var.get_variable(), of_h_var.get_variable(), vars[15].get_variable()],
                    &[],
                )?;

                cs.end_gates_batch_for_step()?;

                Num::Allocated(vars.pop().expect("top element exists"))
            }
        };

        Ok(res)
    }

    fn converter_helper(n: u64, sparse_base: usize, rotation: usize, extraction: usize) -> E::Fr {
        
        let t = map_into_sparse_form(rotate_extract(n as usize, rotation, extraction), sparse_base);
        let mut repr : <E::Fr as PrimeField>::Repr = E::Fr::zero().into_repr();
        repr.as_mut()[0] = t as u64;
        E::Fr::from_repr(repr).expect("should parse")
    }

    fn allocate_converted_num<CS: ConstraintSystem<E>>(
        cs: &mut CS,
        var: &AllocatedNum<E>, 
        chunk_bitlen: usize, 
        chunk_num: usize, 
        sparse_base: usize,
        rotation: usize, 
        extraction: usize
    ) -> Result<AllocatedNum<E>> 
    {
        let new_val = var.get_value().map( |fr| {
            let repr = fr.into_repr();
            let n = (repr.as_ref()[0] >> (chunk_bitlen * chunk_num)) & ((1 << chunk_bitlen) - 1);
            Self::converter_helper(n, sparse_base, rotation, extraction)
        });

        AllocatedNum::alloc(cs, || new_val.grab())
    }

    pub fn query_table1<CS>(cs: &mut CS, table: &Arc<LookupTableApplication<E>>, key: &AllocatedNum<E>) -> Result<AllocatedNum<E>> 
    where CS: ConstraintSystem<E>
    {
        let res = match key.get_value() {
            None => AllocatedNum::alloc(cs, || Err(SynthesisError::AssignmentMissing))?,
            Some(val) => {
                let new_val = table.query(&[val])?[0];
                AllocatedNum::alloc(cs, || Ok(new_val))?
            },     
        };

        cs.begin_gates_batch_for_step()?;

        let dummy = AllocatedNum::alloc_zero(cs)?.get_variable();
        let vars = [key.get_variable(), res.get_variable(), dummy, dummy];
        cs.allocate_variables_without_gate(
            &vars,
            &[]
        )?;
        cs.apply_single_lookup_gate(&vars[..table.width()], table.clone())?;

        cs.end_gates_batch_for_step()?;

        Ok(res)
    }

    pub fn query_table2<CS: ConstraintSystem<E>>(
        cs: &mut CS, 
        table: &Arc<LookupTableApplication<E>>, 
        key: &AllocatedNum<E>
    ) -> Result<(AllocatedNum<E>, AllocatedNum<E>)> 
    {
        let res = match key.get_value() {
            None => (
                AllocatedNum::alloc(cs, || Err(SynthesisError::AssignmentMissing))?, 
                AllocatedNum::alloc(cs, || Err(SynthesisError::AssignmentMissing))?
            ),
            Some(val) => {
                let new_vals = table.query(&[val])?;
                (
                    AllocatedNum::alloc(cs, || Ok(new_vals[0]))?,
                    AllocatedNum::alloc(cs, || Ok(new_vals[1]))?
                )
            },     
        };

        cs.begin_gates_batch_for_step()?;

        let dummy = AllocatedNum::alloc_zero(cs)?.get_variable();
        let vars = [key.get_variable(), res.0.get_variable(), res.1.get_variable(), dummy];
        cs.allocate_variables_without_gate(
            &vars,
            &[]
        )?;
        cs.apply_single_lookup_gate(&vars[..table.width()], table.clone())?;

        cs.end_gates_batch_for_step()?;
        Ok(res)
    }

    // returns n ^ exp if exp is not zero, n otherwise
    fn u64_exp_to_ff(n: u64, exp: u64) -> E::Fr {
        let mut repr : <E::Fr as PrimeField>::Repr = E::Fr::zero().into_repr();
        repr.as_mut()[0] = n;
        let mut res = E::Fr::from_repr(repr).expect("should parse");

        if exp != 0 {
            res = res.pow(&[exp]);
        }

        res
    }

    fn convert_into_sparse_chooser_form<CS : ConstraintSystem<E>>(
        &self, 
        cs: &mut CS, 
        input: NumWithTracker<E>, 
    ) -> Result<SparseChValue<E>> 
    { 
        let var = match input.overflow_tracker {
            OverflowTracker::SignificantOverflow => unimplemented!(),
            OverflowTracker::SmallOverflow => Self::extact_32_from_overflowed_num(cs, &input.num)?,
            _ => input.num,
        };
        
        match var {
            Num::Constant(x) => {
                let repr = x.into_repr();
                // NOTE : think, if it is safe for n to be overflowed
                let n = repr.as_ref()[0] & ((1 << 32) - 1); 
                
                let res = SparseChValue {
                    normal: Num::Constant(x),
                    sparse: Num::Constant(Self::converter_helper(n, SHA256_CHOOSE_BASE, 0, 0)),
                    rot6: Num::Constant(Self::converter_helper(n, SHA256_CHOOSE_BASE, 6, 0)),
                    rot11: Num::Constant(Self::converter_helper(n, SHA256_CHOOSE_BASE, 11, 0)),
                    rot25: Num::Constant(Self::converter_helper(n, SHA256_CHOOSE_BASE, 25, 0)),
                };

                return Ok(res)
            },
            Num::Allocated(var) => {
                
                // split our 32bit variable into 11-bit chunks:
                // there will be three chunks (low, mid, high) for 32bit number
                // note that, we can deal here with possible 1-bit overflow: (as 3 * 11 = 33)
                // in order to do this we allow extraction set to 10 for the table working with highest chunk
                
                let low = Self::allocate_converted_num(cs, &var, SHA256_GADGET_CHUNK_SIZE, 0, 0, 0, 0)?;
                let mid = Self::allocate_converted_num(cs, &var, SHA256_GADGET_CHUNK_SIZE, 1, 0, 0, 0)?;
                let high = Self::allocate_converted_num(cs, &var, SHA256_GADGET_CHUNK_SIZE, 2, 0, 0, SHA256_GADGET_CHUNK_SIZE - 1)?;

                let (sparse_low, sparse_low_rot6) = Self::query_table2(cs, &self.sha256_base7_rot6_table, &low)?;
                let (sparse_mid, _sparse_mid_rot6) = Self::query_table2(cs, &self.sha256_base7_rot6_table, &mid)?;
                let (sparse_high, sparse_high_rot3) = Self::query_table2(cs, &self.sha256_base7_rot3_extr10_table, &high)?;

                let full_normal = {
                    // compose full normal = low + 2^11 * mid + 2^22 * high
                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), Self::u64_exp_to_ff(1 << 11, 0), Self::u64_exp_to_ff(1 << 22, 0)],
                        &[low, mid, high],
                        &var,
                    )?;

                    var.clone()
                };

                let full_sparse = {
                    // full_sparse = low_sparse + 7^11 * mid_sparse + 7^22 * high_sparse
                    let sparse_full = Self::allocate_converted_num(
                        cs, &var, SHA256_REG_WIDTH, 0, SHA256_CHOOSE_BASE, 0, SHA256_REG_WIDTH - 1
                    )?;

                    let limb_1_shift = Self::u64_exp_to_ff(7, 11);
                    let limb_2_shift = Self::u64_exp_to_ff(7, 22);

                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), limb_1_shift, limb_2_shift],
                        &[sparse_low.clone(), sparse_mid.clone(), sparse_high.clone()],
                        &sparse_full,
                    )?;

                    sparse_full
                };

                let full_sparse_rot6 = {
                    // full_sparse_rot6 = low_sparse_rot6 + 7^(11-6) * sparse_mid + 7^(22-6) * sparse_high
                    let full_sparse_rot6 = Self::allocate_converted_num(
                        cs, &var, SHA256_REG_WIDTH, 0, SHA256_CHOOSE_BASE, 6, SHA256_REG_WIDTH - 1
                    )?;

                    let rot6_limb_1_shift = Self::u64_exp_to_ff(7, 11-6);
                    let rot6_limb_2_shift = Self::u64_exp_to_ff(7, 22 - 6);

                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), rot6_limb_1_shift, rot6_limb_2_shift],
                        &[sparse_low_rot6, sparse_mid.clone(), sparse_high.clone()],
                        &full_sparse_rot6,
                    )?;

                    full_sparse_rot6
                };

                let full_sparse_rot11 = {
                    // full_sparse_rot11 = sparse_mid + 7^(22-11) * sparse_high + 7^(32-11) * sparse_low
                    let full_sparse_rot11 = Self::allocate_converted_num(
                        cs, &var, SHA256_REG_WIDTH, 0, SHA256_CHOOSE_BASE, 11, SHA256_REG_WIDTH - 1
                    )?;

                    let rot11_limb_0_shift = Self::u64_exp_to_ff(7, 32 - 11);
                    let rot11_limb_2_shift = Self::u64_exp_to_ff(7, 22 - 11);

                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), rot11_limb_0_shift, rot11_limb_2_shift],
                        &[sparse_mid, sparse_low.clone(), sparse_high.clone()],
                        &full_sparse_rot11,
                    )?;

                    full_sparse_rot11
                };

                let full_sparse_rot_25 = {
                    // full_sparse_rot25 = sparse_high_rot3 + 7^(32-25) * sparse_low + 7^(32-25+11) * sparse_mid
                    let full_sparse_rot25 = Self::allocate_converted_num(
                        cs, &var, SHA256_REG_WIDTH, 0, SHA256_CHOOSE_BASE, 25, SHA256_REG_WIDTH - 1
                    )?;

                    let rot11_limb_0_shift = Self::u64_exp_to_ff(7, 32 - 25);
                    let rot11_limb_2_shift = Self::u64_exp_to_ff(7, 32 - 25 + 11);

                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), rot11_limb_0_shift, rot11_limb_2_shift],
                        &[sparse_high_rot3, sparse_low, sparse_high],
                        &full_sparse_rot25,
                    )?;

                    full_sparse_rot25
                };

                let res = SparseChValue{
                    normal: Num::Allocated(full_normal),
                    sparse: Num::Allocated(full_sparse),
                    rot6: Num::Allocated(full_sparse_rot6),
                    rot11: Num::Allocated(full_sparse_rot11),
                    rot25: Num::Allocated(full_sparse_rot_25),
                };
                return Ok(res);
            }
        }
    }

    // IMPORTANT NOTE:
    // there is a small difference between conversion into sparse chooser form and ... majority form functions 
    // more precisely, we are using 2 different tables in the first case: rot6 table for low and mid chunks and rot3 - for upper one
    // this allows to embed handling of 1-bit overflow into the table itself without additional overflow check (as called above)
    // this works as following: we split our number into  3 11-bit chunks, hence there 33 bits overall
    // however, our upper table for chooser has nontrivial extraction: we forget about the top-most bit of highest chunk, 
    // so our ombined full result will be of length 11 + 11 + 10 = 32, as required
    // NB:
    // 1) this way, we may handle only potential one-bit overflows, for the case of 2-bit overflows and more traditional 
    // approaches are required (as used inside extract_32_from_overflowed_num function)
    // 2) we can use the same approach inside the "conversion into sparse majority form" function - or. in other words, 
    // create two tables instead of one: both will be base4_rot2, but the second one will also containt non-trivial extraction 
    // which forgets about the highest bit of 11-bit chunks. Sometimes, creation of additional goes for free (e.g. in current 
    // implementation, we do not have any penalty in prover's\verifier's workload with the introduction of new table as long as 
    // there total number is less than closest power of 2). The choice of strategy: either work with two tables or work only with
    // base4_rot_2 and ALWAYS do overflow_check (even if we are sure, that we have only one bit of overflow) is handled
    // by MAJORITY_CONVERSION_STRATEGY flag

    fn convert_into_sparse_majority_form<CS : ConstraintSystem<E>>(
        &self, 
        cs: &mut CS, 
        input: NumWithTracker<E>, 
    ) -> Result<SparseMajValue<E>> 
    {      
        let var = match (input.overflow_tracker, self.majority_strategy)  {
            (OverflowTracker::SignificantOverflow, _) => unimplemented!(),
            (OverflowTracker::SmallOverflow, _) | (OverflowTracker::OneBitOverflow, MajorityStrategy::RawOverflowCheck) => {
                Self::extact_32_from_overflowed_num(cs, &input.num)?
            },
            (_, _) => input.num,
        };

        match var {
            Num::Constant(x) => {
                let repr = x.into_repr();
                // NOTE : think, if it is safe for n to be overflowed
                let n = repr.as_ref()[0] & ((1 << 32) - 1); 
                
                let res = SparseMajValue {
                    normal: Num::Constant(x),
                    sparse: Num::Constant(Self::converter_helper(n, SHA256_MAJORITY_BASE, 0, 0)),
                    rot2: Num::Constant(Self::converter_helper(n, SHA256_MAJORITY_BASE, 2, 0)),
                    rot13: Num::Constant(Self::converter_helper(n, SHA256_MAJORITY_BASE, 13, 0)),
                    rot22: Num::Constant(Self::converter_helper(n, SHA256_MAJORITY_BASE, 22, 0)),
                };

                return Ok(res)
            },
            Num::Allocated(var) => {
                
                // split our 32bit variable into 11-bit chunks:
                // there will be three chunks (low, mid, high) for 32bit number
                // note that, we can deal here with possible 1-bit overflow: (as 3 * 11 = 33)
                // in order to do this we allow extraction set to 10 for the table working with highest chunk
                
                let low = Self::allocate_converted_num(cs, &var, SHA256_GADGET_CHUNK_SIZE, 0, 0, 0, 0)?;
                let mid = Self::allocate_converted_num(cs, &var, SHA256_GADGET_CHUNK_SIZE, 1, 0, 0, 0)?;
                let high = Self::allocate_converted_num(cs, &var, SHA256_GADGET_CHUNK_SIZE, 2, 0, 0, SHA256_GADGET_CHUNK_SIZE - 1)?;

                let (sparse_low, sparse_low_rot2) = Self::query_table2(cs, &self.sha256_base4_rot2_table, &low)?;
                let (sparse_mid, sparse_mid_rot2) = Self::query_table2(cs, &self.sha256_base4_rot2_table, &mid)?;
                let high_chunk_table = match self.majority_strategy {
                    MajorityStrategy::UseTwoTables => self.sha256_base4_rot2_extr10_table.as_ref().unwrap(),
                    MajorityStrategy::RawOverflowCheck => &self.sha256_base4_rot2_table,
                };
                let (sparse_high, _sparse_high_rot2) = Self::query_table2(cs, high_chunk_table, &high)?;

                let full_normal = {
                    // compose full normal = low + 2^11 * mid + 2^22 * high
                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), Self::u64_exp_to_ff(1 << 11, 0), Self::u64_exp_to_ff(1 << 22, 0)],
                        &[low, mid, high],
                        &var,
                    )?;

                    var.clone()
                };

                let full_sparse = {
                    // full_sparse = low_sparse + 4^11 * mid_sparse + 4^22 * high_sparse
                    let sparse_full = Self::allocate_converted_num(
                        cs, &var, SHA256_REG_WIDTH, 0, SHA256_MAJORITY_BASE, 0, SHA256_REG_WIDTH - 1
                    )?;

                    let limb_1_shift = Self::u64_exp_to_ff(4, 11);
                    let limb_2_shift = Self::u64_exp_to_ff(4, 22);

                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), limb_1_shift, limb_2_shift],
                        &[sparse_low.clone(), sparse_mid.clone(), sparse_high.clone()],
                        &sparse_full,
                    )?;

                    sparse_full
                };

                let full_sparse_rot2 = {
                    // full_sparse_rot6 = low_sparse_rot2 + 4^(11-2) * sparse_mid + 4^(22-2) * sparse_high
                    let full_sparse_rot2 = Self::allocate_converted_num(
                        cs, &var, SHA256_REG_WIDTH, 0, SHA256_CHOOSE_BASE, 2, SHA256_REG_WIDTH - 1
                    )?;

                    let rot2_limb_1_shift = Self::u64_exp_to_ff(4, 11-2);
                    let rot2_limb_2_shift = Self::u64_exp_to_ff(4, 22 - 6);

                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), rot2_limb_1_shift, rot2_limb_2_shift],
                        &[sparse_low_rot2, sparse_mid.clone(), sparse_high.clone()],
                        &full_sparse_rot2,
                    )?;

                    full_sparse_rot2
                };

                let full_sparse_rot13 = {
                    // full_sparse_rot13 = sparse_mid_rot2 + 4^(22-11-2) * sparse_high + 4^(32-11-2) * sparse_low
                    let full_sparse_rot13 = Self::allocate_converted_num(
                        cs, &var, SHA256_REG_WIDTH, 0, SHA256_CHOOSE_BASE, 13, SHA256_REG_WIDTH - 1
                    )?;

                    let rot13_limb_0_shift = Self::u64_exp_to_ff(4, 32 - 2 - 11);
                    let rot13_limb_2_shift = Self::u64_exp_to_ff(4, 22 - 2 - 11);

                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), rot13_limb_0_shift, rot13_limb_2_shift],
                        &[sparse_mid_rot2, sparse_low.clone(), sparse_high.clone()],
                        &full_sparse_rot13,
                    )?;

                    full_sparse_rot13
                };

                let full_sparse_rot_22 = {
                    // full_sparse_rot22 = sparse_high + 4^(32 - 22) * sparse_low + 4^(32 - 22 + 11) * sparse_mid
                    let full_sparse_rot22 = Self::allocate_converted_num(
                        cs, &var, SHA256_REG_WIDTH, 0, SHA256_CHOOSE_BASE, 22, SHA256_REG_WIDTH - 1
                    )?;

                    let rot22_limb_0_shift = Self::u64_exp_to_ff(4, 32 - 22);
                    let rot22_limb_1_shift = Self::u64_exp_to_ff(4, 32 - 22 + 11);

                    AllocatedNum::ternary_lc_eq(
                        cs, 
                        &[E::Fr::one(), rot22_limb_0_shift, rot22_limb_1_shift],
                        &[sparse_high, sparse_low, sparse_mid],
                        &full_sparse_rot22,
                    )?;

                    full_sparse_rot22
                };

                let res = SparseMajValue{
                    normal: Num::Allocated(full_normal),
                    sparse: Num::Allocated(full_sparse),
                    rot2: Num::Allocated(full_sparse_rot2),
                    rot13: Num::Allocated(full_sparse_rot13),
                    rot22: Num::Allocated(full_sparse_rot_22),
                };
                return Ok(res);
            }
        }
    }

    // this function does the following: 
    // given any x = \sum_{i=0}^{n} x_i * base^i and well-defined mapping f: [0; base-1] -> [0; 1]
    // (among possible variants for f are "parity": f(k) = k mod 2, choose_function or majority_function:
    // for the description of the latter two refer to "tables" module)
    // return the "normalized" variable z = \sum_{i=0}^{n} f(x_i) 2^i
    //
    // the problem with table approach is actually the following:
    // we are unable to create long table holding all possible values of x in the range [0; base^n - 1] (granting that n is fixed)
    // the reason is that we do not want our tables to be EXTREMELY large, hence we require one additional step of workaround:
    // given adjustible parameter NUM_OF_CHUNKS we split our x in the linear combination of [ n / NUM_OF_CHUNKS] summands y_i,
    // each of which itself consists of no more than NUM_OF_CHUNKS summands
    //
    // in other words, we have:
    // x = \sum_{j=0}^{[n / NUM_OF_CHUNKS]} y_j * base^{j * NUM_OF_CHUNKS},
    // where y_j = \sum_{i=0}^{NUM_CHUNKS - 1} x_{j * NUM_OF_CHUNKS + x_i} * base^{j}
    // each such y_j is in smaller range [0; base^NUM_CHUNKS-1]
    // and for each such y_j we may apply the corresponding (and realtively small) normalization table and get
    // z_j = \sum_{i=0}^{NUM_CHUNKS} f(x_{j * NUM_OF_CHUNKS + x_i}) 2^j
    // the final z is then constructed as a linear conbination of {z_j} with simple weigt coefficients 
    // (in order for each z_j to be placed in an appropriate position in the bit representation of final result z)
    //
    // note, that for each possible pair of normalization transformation f(x) and base,
    // the parameter NUM_OF_CHUNKS may be determined separately
    // 
    // e.g. in reference implementation Barretenberg a = NUM_OF_CHUNKS = 4 for base = 7 and b = NUM_OF_CHUNKS = 6 for base = 4
    // IMHO, the motivation for such choice of parameters is the following:
    // in any case we would use sparse_rotate_extract tables with 11-bit chunks (and hence of size c = 2^11)
    // parameters a and b are chosen in a way, so that table sizes for normalization transforms of sizes 7^a and 4^b
    // approximately have the same order of magnitude as c, so that all used tables will be of relatively the same size
    // it is obvious, that following this logic, a = 4 and b = 6 (or may be 5(!)) are best possible choices
    //
    // in any case we do not want to be two strict here, and allow NUM_OF_CHUNKS for bases 7 and 4
    // to be specified as constructor parameters for Sha256Gadget gadget

    fn normalize<CS>(cs: &mut CS, input: &Num<E>, table: &Arc<LookupTableApplication<E>>, base: usize, num_chunks: usize) -> Num<E>
    where CS: ConstraintSystem<E>
    {
        match input {
            Num::Constant(x) => {
                let output = table.query(&[val])?[0];
                return Num::Constant(output);
            }
            Num::Allocated(x) => {
                // split and slice!
            }
        }

       

        uint64_t base_product = 1;
        uint64_t binary_product = 1 << num_bits;
        for (size_t i = 0; i < num_bits; ++i) {
            base_product *= base;
        }
        const uint256_t slice_maximum(base_product);

        constexpr size_t num_slices = (32 / num_bits) + ((num_bits % num_bits) == 0);
        std::array<field_t<waffle::PLookupComposer>, num_slices> input_slices;
        for (auto& slice : input_slices) {
            uint64_t witness = (sparse % slice_maximum).data[0];
            slice = witness_t<waffle::PLookupComposer>(ctx, barretenberg::fr(witness));
            sparse /= slice_maximum;
        }

        std::array<field_t<waffle::PLookupComposer>, num_slices> output_slices;
        for (size_t i = 0; i < num_slices; ++i) {
            output_slices[i] = field_t<waffle::PLookupComposer>(ctx);
            output_slices[i].witness_index = ctx->read_from_table(table_id, input_slices[i].witness_index);
        }

        field_t<waffle::PLookupComposer> input_sum = input_slices[0];
        field_t<waffle::PLookupComposer> output_sum = output_slices[0];

        field_t<waffle::PLookupComposer> sparse_base(ctx, base_product);
        field_t<waffle::PLookupComposer> sparse_base_accumulator = sparse_base;

        field_t<waffle::PLookupComposer> base2(ctx, binary_product);
        field_t<waffle::PLookupComposer> base2_accumulator = base2;

        for (size_t i = 1; i < num_slices - 1; i += 2) {
            const auto t1 = sparse_base_accumulator * sparse_base;
            input_sum = input_sum.add_two(input_slices[i] * sparse_base_accumulator, input_slices[i + 1] * t1);
            sparse_base_accumulator = t1 * sparse_base;

            const auto t2 = base2_accumulator * base2;
            output_sum = output_sum.add_two(output_slices[i] * base2_accumulator, output_slices[i + 1] * t2);
            base2_accumulator = t2 * base2;
        }
        if ((num_slices & 1) == 0) {
            input_sum += input_slices[num_slices - 1] * sparse_base_accumulator;
            output_sum += output_slices[num_slices - 1] * base2_accumulator;
        }

        return output_sum;
    }


//     fn choose<CS: ConstraintSystem<E>>(cs: &mut CS, e: SparseChValue<E>, f: SparseChValue<E>, g: SparseChValue<E>) -> Num<E>
//     {
        
//         let mut two = E::Fr::one();
//         two.double();
//         let mut three = two.clone();
//         three.add_asign(&E::Fr::one());
        
//         let t0 = Num::lc(cs, coeffs: &[E:Fr::one(), E::Fr::two(), E::Fr::three()], nums: &[e.sparse, f.sparse, g.sparse])  e.sparse.add_two(f.sparse + f.sparse, g.sparse + g.sparse + g.sparse);
//         const auto t1 = e.rot6.add_two(e.rot11, e.rot25);

//         const auto r0 = normalize_sparse_form<7, 4>(t0, waffle::PLookupTableId::SHA256_PARTA_NORMALIZE);
//         const auto r1 = normalize_sparse_form<7, 4>(t1, waffle::PLookupTableId::SHA256_BASE7_NORMALIZE);

//         return r0 + r1;
//     }

// field_t<waffle::PLookupComposer> majority(const sparse_maj_value& a,
//                                           const sparse_maj_value& b,
//                                           const sparse_maj_value& c)
// {
//     const auto t0 = a.sparse.add_two(b.sparse, c.sparse);
//     const auto t1 = a.rot2.add_two(a.rot13, a.rot22);

//     const auto r0 = normalize_sparse_form<4, 6>(t0, waffle::PLookupTableId::SHA256_PARTB_NORMALIZE);
//     const auto r1 = normalize_sparse_form<4, 6>(t1, waffle::PLookupTableId::SHA256_BASE4_NORMALIZE);

//     return r0 + r1;
// }
}
   






