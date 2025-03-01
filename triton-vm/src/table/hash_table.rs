use itertools::Itertools;
use ndarray::s;
use ndarray::Array1;
use ndarray::ArrayView1;
use ndarray::ArrayView2;
use ndarray::ArrayViewMut2;
use num_traits::One;
use strum::EnumCount;
use strum_macros::Display;
use strum_macros::EnumCount as EnumCountMacro;
use strum_macros::EnumIter;
use twenty_first::shared_math::b_field_element::BFieldElement;
use twenty_first::shared_math::b_field_element::BFIELD_ONE;
use twenty_first::shared_math::rescue_prime_digest::DIGEST_LENGTH;
use twenty_first::shared_math::rescue_prime_regular::ALPHA;
use twenty_first::shared_math::rescue_prime_regular::MDS;
use twenty_first::shared_math::rescue_prime_regular::MDS_INV;
use twenty_first::shared_math::rescue_prime_regular::NUM_ROUNDS;
use twenty_first::shared_math::rescue_prime_regular::RATE;
use twenty_first::shared_math::rescue_prime_regular::ROUND_CONSTANTS;
use twenty_first::shared_math::rescue_prime_regular::STATE_SIZE;
use twenty_first::shared_math::x_field_element::XFieldElement;

use triton_opcodes::instruction::Instruction;

use crate::table::challenges::TableChallenges;
use crate::table::constraint_circuit::ConstraintCircuit;
use crate::table::constraint_circuit::ConstraintCircuitBuilder;
use crate::table::constraint_circuit::ConstraintCircuitMonad;
use crate::table::constraint_circuit::DualRowIndicator;
use crate::table::constraint_circuit::DualRowIndicator::*;
use crate::table::constraint_circuit::InputIndicator;
use crate::table::constraint_circuit::SingleRowIndicator;
use crate::table::constraint_circuit::SingleRowIndicator::*;
use crate::table::cross_table_argument::CrossTableArg;
use crate::table::cross_table_argument::EvalArg;
use crate::table::hash_table::HashTableChallengeId::*;
use crate::table::master_table::NUM_BASE_COLUMNS;
use crate::table::master_table::NUM_EXT_COLUMNS;
use crate::table::table_column::BaseTableColumn;
use crate::table::table_column::ExtTableColumn;
use crate::table::table_column::HashBaseTableColumn;
use crate::table::table_column::HashBaseTableColumn::*;
use crate::table::table_column::HashExtTableColumn;
use crate::table::table_column::HashExtTableColumn::*;
use crate::table::table_column::MasterBaseTableColumn;
use crate::table::table_column::MasterExtTableColumn;
use crate::vm::AlgebraicExecutionTrace;

pub const HASH_TABLE_NUM_PERMUTATION_ARGUMENTS: usize = 0;
pub const HASH_TABLE_NUM_EVALUATION_ARGUMENTS: usize = 2;
pub const HASH_TABLE_NUM_EXTENSION_CHALLENGES: usize = HashTableChallengeId::COUNT;

pub const BASE_WIDTH: usize = HashBaseTableColumn::COUNT;
pub const EXT_WIDTH: usize = HashExtTableColumn::COUNT;
pub const FULL_WIDTH: usize = BASE_WIDTH + EXT_WIDTH;

pub const NUM_ROUND_CONSTANTS: usize = STATE_SIZE * 2;
pub const TOTAL_NUM_CONSTANTS: usize = NUM_ROUND_CONSTANTS * NUM_ROUNDS;

#[derive(Debug, Clone)]
pub struct HashTable {}

#[derive(Debug, Clone)]
pub struct ExtHashTable {}

impl ExtHashTable {
    pub fn ext_initial_constraints_as_circuits() -> Vec<
        ConstraintCircuit<
            HashTableChallenges,
            SingleRowIndicator<NUM_BASE_COLUMNS, NUM_EXT_COLUMNS>,
        >,
    > {
        let circuit_builder = ConstraintCircuitBuilder::new();
        let challenge = |c| circuit_builder.challenge(c);
        let constant = |c| circuit_builder.b_constant(c);
        let one = constant(BFIELD_ONE);

        let base_row = |column: HashBaseTableColumn| {
            circuit_builder.input(BaseRow(column.master_base_table_index()))
        };
        let ext_row = |column: HashExtTableColumn| {
            circuit_builder.input(ExtRow(column.master_ext_table_index()))
        };

        let running_evaluation_initial = circuit_builder.x_constant(EvalArg::default_initial());

        let round_number = base_row(ROUNDNUMBER);
        let ci = base_row(CI);
        let running_evaluation_hash_input = ext_row(HashInputRunningEvaluation);
        let running_evaluation_hash_digest = ext_row(HashDigestRunningEvaluation);
        let running_evaluation_sponge = ext_row(SpongeRunningEvaluation);

        let ci_is_hash = ci.clone() - constant(Instruction::Hash.opcode_b());
        let ci_is_absorb_init = ci - constant(Instruction::AbsorbInit.opcode_b());
        let state = [
            STATE0, STATE1, STATE2, STATE3, STATE4, STATE5, STATE6, STATE7, STATE8, STATE9,
        ];
        let state_weights = [
            HashStateWeight0,
            HashStateWeight1,
            HashStateWeight2,
            HashStateWeight3,
            HashStateWeight4,
            HashStateWeight5,
            HashStateWeight6,
            HashStateWeight7,
            HashStateWeight8,
            HashStateWeight9,
        ];
        let compressed_row: ConstraintCircuitMonad<_, _> = state_weights
            .into_iter()
            .zip_eq(state.into_iter())
            .map(|(weight, state)| challenge(weight) * base_row(state))
            .sum();

        let round_number_is_0_or_1 = round_number.clone() * (round_number.clone() - one.clone());

        let current_instruction_is_absorb_init_or_hash =
            ci_is_absorb_init.clone() * ci_is_hash.clone();

        // Evaluation Argument “hash input”
        // If the round number is 0, the running evaluation is the default initial.
        // If the current instruction is AbsorbInit, the running evaluation is the default initial.
        // Else, the first update has been applied to the running evaluation.
        let from_processor_indeterminate = challenge(HashInputEvalIndeterminate);
        let running_evaluation_hash_input_is_default_initial =
            running_evaluation_hash_input.clone() - running_evaluation_initial.clone();
        let running_evaluation_hash_input_has_accumulated_first_row = running_evaluation_hash_input
            - running_evaluation_initial.clone() * from_processor_indeterminate
            - compressed_row.clone();
        let running_evaluation_hash_input_is_initialized_correctly = round_number.clone()
            * ci_is_absorb_init.clone()
            * running_evaluation_hash_input_has_accumulated_first_row
            + ci_is_hash.clone() * running_evaluation_hash_input_is_default_initial.clone()
            + (one - round_number) * running_evaluation_hash_input_is_default_initial;

        // Evaluation Argument “hash digest”
        let running_evaluation_hash_digest_is_default_initial =
            running_evaluation_hash_digest - running_evaluation_initial.clone();

        // Evaluation Argument “Sponge”
        let sponge_indeterminate = challenge(SpongeEvalIndeterminate);
        let running_evaluation_sponge_is_default_initial =
            running_evaluation_sponge.clone() - running_evaluation_initial.clone();
        let running_evaluation_sponge_has_accumulated_first_row = running_evaluation_sponge
            - running_evaluation_initial * sponge_indeterminate
            - challenge(CIWeight) * constant(Instruction::AbsorbInit.opcode_b())
            - compressed_row;
        let running_evaluation_sponge_absorb_is_initialized_correctly = ci_is_hash
            * running_evaluation_sponge_has_accumulated_first_row
            + ci_is_absorb_init * running_evaluation_sponge_is_default_initial;

        [
            round_number_is_0_or_1,
            current_instruction_is_absorb_init_or_hash,
            running_evaluation_hash_input_is_initialized_correctly,
            running_evaluation_hash_digest_is_default_initial,
            running_evaluation_sponge_absorb_is_initialized_correctly,
        ]
        .map(|circuit| circuit.consume())
        .to_vec()
    }

    fn round_number_deselector<II: InputIndicator>(
        circuit_builder: &ConstraintCircuitBuilder<HashTableChallenges, II>,
        round_number_circuit_node: &ConstraintCircuitMonad<HashTableChallenges, II>,
        round_number_to_deselect: usize,
    ) -> ConstraintCircuitMonad<HashTableChallenges, II> {
        let constant = |c: u64| circuit_builder.b_constant(c.into());
        (0..=NUM_ROUNDS + 1)
            .filter(|&r| r != round_number_to_deselect)
            .map(|r| round_number_circuit_node.clone() - constant(r as u64))
            .fold(constant(1), |a, b| a * b)
    }

    pub fn ext_consistency_constraints_as_circuits() -> Vec<
        ConstraintCircuit<
            HashTableChallenges,
            SingleRowIndicator<NUM_BASE_COLUMNS, NUM_EXT_COLUMNS>,
        >,
    > {
        let circuit_builder = ConstraintCircuitBuilder::new();
        let constant = |c: u64| circuit_builder.b_constant(c.into());

        let round_number = circuit_builder.input(BaseRow(ROUNDNUMBER.master_base_table_index()));
        let ci = circuit_builder.input(BaseRow(CI.master_base_table_index()));
        let state10 = circuit_builder.input(BaseRow(STATE10.master_base_table_index()));
        let state11 = circuit_builder.input(BaseRow(STATE11.master_base_table_index()));
        let state12 = circuit_builder.input(BaseRow(STATE12.master_base_table_index()));
        let state13 = circuit_builder.input(BaseRow(STATE13.master_base_table_index()));
        let state14 = circuit_builder.input(BaseRow(STATE14.master_base_table_index()));
        let state15 = circuit_builder.input(BaseRow(STATE15.master_base_table_index()));

        let ci_is_hash = ci.clone() - constant(Instruction::Hash.opcode() as u64);
        let ci_is_absorb_init = ci.clone() - constant(Instruction::AbsorbInit.opcode() as u64);
        let ci_is_absorb = ci.clone() - constant(Instruction::Absorb.opcode() as u64);
        let ci_is_squeeze = ci - constant(Instruction::Squeeze.opcode() as u64);

        let round_number_is_not_0 =
            Self::round_number_deselector(&circuit_builder, &round_number, 0);
        let round_number_is_not_1 =
            Self::round_number_deselector(&circuit_builder, &round_number, 1);
        let mut consistency_constraint_circuits = vec![
            round_number_is_not_0 * ci_is_hash.clone(),
            round_number_is_not_1.clone()
                * ci_is_absorb_init
                * ci_is_absorb.clone()
                * ci_is_squeeze.clone()
                * (state10.clone() - constant(1)),
            round_number_is_not_1.clone()
                * ci_is_hash
                * ci_is_absorb.clone()
                * ci_is_squeeze.clone()
                * state10,
            round_number_is_not_1.clone() * ci_is_absorb.clone() * ci_is_squeeze.clone() * state11,
            round_number_is_not_1.clone() * ci_is_absorb.clone() * ci_is_squeeze.clone() * state12,
            round_number_is_not_1.clone() * ci_is_absorb.clone() * ci_is_squeeze.clone() * state13,
            round_number_is_not_1.clone() * ci_is_absorb.clone() * ci_is_squeeze.clone() * state14,
            round_number_is_not_1 * ci_is_absorb * ci_is_squeeze * state15,
        ];

        let round_constant_offset = CONSTANT0A.master_base_table_index();
        for round_constant_col_index in 0..NUM_ROUND_CONSTANTS {
            let round_constant_input =
                circuit_builder.input(BaseRow(round_constant_col_index + round_constant_offset));
            let round_constant_constraint_circuit = (1..=NUM_ROUNDS)
                .map(|i| {
                    let round_constant_idx =
                        NUM_ROUND_CONSTANTS * (i - 1) + round_constant_col_index;
                    Self::round_number_deselector(&circuit_builder, &round_number, i)
                        * (round_constant_input.clone()
                            - circuit_builder.b_constant(ROUND_CONSTANTS[round_constant_idx]))
                })
                .sum();
            consistency_constraint_circuits.push(round_constant_constraint_circuit);
        }

        consistency_constraint_circuits
            .into_iter()
            .map(|circuit| circuit.consume())
            .collect()
    }

    pub fn ext_transition_constraints_as_circuits() -> Vec<
        ConstraintCircuit<HashTableChallenges, DualRowIndicator<NUM_BASE_COLUMNS, NUM_EXT_COLUMNS>>,
    > {
        let circuit_builder = ConstraintCircuitBuilder::new();
        let challenge = |c| circuit_builder.challenge(c);
        let constant = |c: u64| circuit_builder.b_constant(c.into());
        let b_constant = |c| circuit_builder.b_constant(c);

        let opcode_hash = constant(Instruction::Hash.opcode() as u64);
        let opcode_absorb_init = constant(Instruction::AbsorbInit.opcode() as u64);
        let opcode_absorb = constant(Instruction::Absorb.opcode() as u64);
        let opcode_squeeze = constant(Instruction::Squeeze.opcode() as u64);

        let current_base_row = |column_idx: HashBaseTableColumn| {
            circuit_builder.input(CurrentBaseRow(column_idx.master_base_table_index()))
        };
        let next_base_row = |column_idx: HashBaseTableColumn| {
            circuit_builder.input(NextBaseRow(column_idx.master_base_table_index()))
        };
        let current_ext_row = |column_idx: HashExtTableColumn| {
            circuit_builder.input(CurrentExtRow(column_idx.master_ext_table_index()))
        };
        let next_ext_row = |column_idx: HashExtTableColumn| {
            circuit_builder.input(NextExtRow(column_idx.master_ext_table_index()))
        };

        let hash_input_eval_indeterminate = challenge(HashInputEvalIndeterminate);
        let hash_digest_eval_indeterminate = challenge(HashDigestEvalIndeterminate);
        let sponge_indeterminate = challenge(SpongeEvalIndeterminate);

        let round_number = current_base_row(ROUNDNUMBER);
        let ci = current_base_row(CI);
        let running_evaluation_hash_input = current_ext_row(HashInputRunningEvaluation);
        let running_evaluation_hash_digest = current_ext_row(HashDigestRunningEvaluation);
        let running_evaluation_sponge = current_ext_row(SpongeRunningEvaluation);

        let round_number_next = next_base_row(ROUNDNUMBER);
        let ci_next = next_base_row(CI);
        let running_evaluation_hash_input_next = next_ext_row(HashInputRunningEvaluation);
        let running_evaluation_hash_digest_next = next_ext_row(HashDigestRunningEvaluation);
        let running_evaluation_sponge_next = next_ext_row(SpongeRunningEvaluation);

        let state: [_; STATE_SIZE] = [
            STATE0, STATE1, STATE2, STATE3, STATE4, STATE5, STATE6, STATE7, STATE8, STATE9,
            STATE10, STATE11, STATE12, STATE13, STATE14, STATE15,
        ];
        let state_current = state.map(current_base_row);
        let state_next = state.map(next_base_row);

        let state_weights = [
            HashStateWeight0,
            HashStateWeight1,
            HashStateWeight2,
            HashStateWeight3,
            HashStateWeight4,
            HashStateWeight5,
            HashStateWeight6,
            HashStateWeight7,
            HashStateWeight8,
            HashStateWeight9,
            HashStateWeight10,
            HashStateWeight11,
            HashStateWeight12,
            HashStateWeight13,
            HashStateWeight14,
            HashStateWeight15,
        ]
        .map(challenge);

        // round number
        // round numbers evolve as
        // 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7 -> 8 -> 9, and
        // 9 -> 1 or 9 -> 0, and
        // 0 -> 0

        let round_number_is_not_0 =
            Self::round_number_deselector(&circuit_builder, &round_number, 0);
        let round_number_is_not_9 =
            Self::round_number_deselector(&circuit_builder, &round_number, 9);

        // if round number is 0, then next round number is 0
        // DNF: rn in {1, ..., 9} ∨ rn* = 0
        let round_number_is_1_through_9_or_round_number_next_is_0 =
            round_number_is_not_0 * round_number_next.clone();

        // if round number is 9, then next round number is 0 or 1
        // DNF: rn =/= 9 ∨ rn* = 0 ∨ rn* = 1
        let round_number_is_0_through_8_or_round_number_next_is_0_or_1 = round_number_is_not_9
            * (constant(1) - round_number_next.clone())
            * round_number_next.clone();

        // if round number is in {1, ..., 8} then next round number is +1
        // DNF: (rn == 0 ∨ rn == 9) ∨ rn* = rn + 1
        let round_number_is_0_or_9_or_increments_by_one = round_number.clone()
            * (constant(NUM_ROUNDS as u64 + 1) - round_number.clone())
            * (round_number_next.clone() - round_number.clone() - constant(1));

        let if_ci_is_hash_then_ci_doesnt_change = (ci.clone() - opcode_absorb_init.clone())
            * (ci.clone() - opcode_absorb.clone())
            * (ci.clone() - opcode_squeeze.clone())
            * (ci_next.clone() - opcode_hash.clone());

        let if_round_number_is_not_9_then_ci_doesnt_change =
            (round_number.clone() - constant(NUM_ROUNDS as u64 + 1)) * (ci_next.clone() - ci);

        // Rescue-XLIX

        let round_constants_a: [_; STATE_SIZE] = [
            CONSTANT0A,
            CONSTANT1A,
            CONSTANT2A,
            CONSTANT3A,
            CONSTANT4A,
            CONSTANT5A,
            CONSTANT6A,
            CONSTANT7A,
            CONSTANT8A,
            CONSTANT9A,
            CONSTANT10A,
            CONSTANT11A,
            CONSTANT12A,
            CONSTANT13A,
            CONSTANT14A,
            CONSTANT15A,
        ]
        .map(current_base_row);
        let round_constants_b: [_; STATE_SIZE] = [
            CONSTANT0B,
            CONSTANT1B,
            CONSTANT2B,
            CONSTANT3B,
            CONSTANT4B,
            CONSTANT5B,
            CONSTANT6B,
            CONSTANT7B,
            CONSTANT8B,
            CONSTANT9B,
            CONSTANT10B,
            CONSTANT11B,
            CONSTANT12B,
            CONSTANT13B,
            CONSTANT14B,
            CONSTANT15B,
        ]
        .map(current_base_row);

        // left-hand-side, starting at current round and going forward

        let after_sbox = {
            let mut exponentiation_accumulator = state_current.to_vec();
            for _ in 1..ALPHA {
                for i in 0..exponentiation_accumulator.len() {
                    exponentiation_accumulator[i] =
                        exponentiation_accumulator[i].clone() * state_current[i].clone();
                }
            }
            exponentiation_accumulator
        };
        let after_mds = (0..STATE_SIZE)
            .map(|i| {
                (0..STATE_SIZE)
                    .map(|j| b_constant(MDS[i * STATE_SIZE + j]) * after_sbox[j].clone())
                    .sum::<ConstraintCircuitMonad<_, _>>()
            })
            .collect_vec();

        let after_constants = after_mds
            .into_iter()
            .zip_eq(round_constants_a)
            .map(|(st, rndc)| st + rndc)
            .collect_vec();

        // right hand side; move backwards
        let before_constants = state_next
            .clone()
            .into_iter()
            .zip_eq(round_constants_b)
            .map(|(st, rndc)| st - rndc)
            .collect_vec();
        let before_mds = (0..STATE_SIZE)
            .map(|i| {
                (0..STATE_SIZE)
                    .map(|j| b_constant(MDS_INV[i * STATE_SIZE + j]) * before_constants[j].clone())
                    .sum::<ConstraintCircuitMonad<_, _>>()
            })
            .collect_vec();

        let before_sbox = {
            let mut exponentiation_accumulator = before_mds.clone();
            for _ in 1..ALPHA {
                for i in 0..exponentiation_accumulator.len() {
                    exponentiation_accumulator[i] =
                        exponentiation_accumulator[i].clone() * before_mds[i].clone();
                }
            }
            exponentiation_accumulator
        };

        // Equate left hand side to right hand side. Ignore if padding row or after final round.

        let hash_function_round_correctly_performs_update = after_constants
            .into_iter()
            .zip_eq(before_sbox.into_iter())
            .map(|(lhs, rhs)| {
                round_number.clone()
                    * (round_number.clone() - constant(NUM_ROUNDS as u64 + 1))
                    * (lhs - rhs)
            })
            .collect_vec();

        // copy capacity between rounds with index 9 and 1 if instruction is “absorb”
        let round_number_next_is_not_1 =
            Self::round_number_deselector(&circuit_builder, &round_number_next, 1);
        let round_number_next_is_1 = round_number_next.clone() - constant(1);

        let difference_of_capacity_registers = state_current[RATE..]
            .iter()
            .zip_eq(state_next[RATE..].iter())
            .map(|(current, next)| next.clone() - current.clone())
            .collect_vec();
        let randomized_sum_of_capacity_differences = state_weights[RATE..]
            .iter()
            .zip_eq(difference_of_capacity_registers)
            .map(|(weight, state_difference)| weight.clone() * state_difference)
            .sum();
        let if_round_number_next_is_1_and_ci_next_is_absorb_then_capacity_doesnt_change =
            round_number_next_is_not_1.clone()
                * (ci_next.clone() - opcode_hash.clone())
                * (ci_next.clone() - opcode_absorb_init.clone())
                * (ci_next.clone() - opcode_squeeze.clone())
                * randomized_sum_of_capacity_differences;

        // copy entire state between rounds with index 9 and 1 if instruction is “squeeze”
        let difference_of_state_registers = state_current
            .iter()
            .zip_eq(state_next.iter())
            .map(|(current, next)| next.clone() - current.clone())
            .collect_vec();
        let randomized_sum_of_state_differences = state_weights
            .iter()
            .zip_eq(difference_of_state_registers.iter())
            .map(|(weight, state_difference)| weight.clone() * state_difference.clone())
            .sum();
        let if_round_number_next_is_1_and_ci_next_is_squeeze_then_state_doesnt_change =
            round_number_next_is_not_1.clone()
                * (ci_next.clone() - opcode_hash.clone())
                * (ci_next.clone() - opcode_absorb_init.clone())
                * (ci_next.clone() - opcode_absorb.clone())
                * randomized_sum_of_state_differences;

        // Evaluation Arguments

        // If (and only if) the next row number is 1, update running evaluation “hash input.”
        let ci_next_is_not_hash = (ci_next.clone() - opcode_absorb_init.clone())
            * (ci_next.clone() - opcode_absorb.clone())
            * (ci_next.clone() - opcode_squeeze.clone());
        let running_evaluation_hash_input_remains =
            running_evaluation_hash_input_next.clone() - running_evaluation_hash_input.clone();
        let xlix_input = state_next[0..2 * DIGEST_LENGTH].to_owned();
        let compressed_row_from_processor = xlix_input
            .into_iter()
            .zip_eq(state_weights[0..2 * DIGEST_LENGTH].iter())
            .map(|(state, weight)| weight.clone() * state)
            .sum();

        let running_evaluation_hash_input_updates = running_evaluation_hash_input_next
            - hash_input_eval_indeterminate * running_evaluation_hash_input
            - compressed_row_from_processor;
        let running_evaluation_hash_input_is_updated_correctly = round_number_next_is_not_1.clone()
            * ci_next_is_not_hash.clone()
            * running_evaluation_hash_input_updates
            + round_number_next_is_1.clone() * running_evaluation_hash_input_remains.clone()
            + (ci_next.clone() - opcode_hash.clone()) * running_evaluation_hash_input_remains;

        // If (and only if) the next row number is 9, update running evaluation “hash digest.”
        let round_number_next_is_9 = round_number_next.clone() - constant(NUM_ROUNDS as u64 + 1);
        let round_number_next_is_not_9 =
            Self::round_number_deselector(&circuit_builder, &round_number_next, NUM_ROUNDS + 1);
        let running_evaluation_hash_digest_remains =
            running_evaluation_hash_digest_next.clone() - running_evaluation_hash_digest.clone();
        let hash_digest = state_next[0..DIGEST_LENGTH].to_owned();
        let compressed_row_hash_digest = hash_digest
            .into_iter()
            .zip_eq(state_weights[0..DIGEST_LENGTH].iter())
            .map(|(state, weight)| weight.clone() * state)
            .sum();
        let running_evaluation_hash_digest_updates = running_evaluation_hash_digest_next
            - hash_digest_eval_indeterminate * running_evaluation_hash_digest
            - compressed_row_hash_digest;
        let running_evaluation_hash_digest_is_updated_correctly = round_number_next_is_not_9
            * ci_next_is_not_hash
            * running_evaluation_hash_digest_updates
            + round_number_next_is_9 * running_evaluation_hash_digest_remains.clone()
            + (ci_next.clone() - opcode_hash.clone()) * running_evaluation_hash_digest_remains;

        // The running evaluation for “Sponge” updates correctly.
        let compressed_row_next = state_weights[..RATE]
            .iter()
            .zip_eq(state_next[..RATE].iter())
            .map(|(weight, st_next)| weight.clone() * st_next.clone())
            .sum();
        let running_evaluation_sponge_has_accumulated_next_row = running_evaluation_sponge_next
            .clone()
            - sponge_indeterminate.clone() * running_evaluation_sponge.clone()
            - challenge(CIWeight) * ci_next.clone()
            - compressed_row_next;
        let if_round_no_next_1_and_ci_next_absorb_init_or_squeeze_then_running_eval_sponge_updates =
            round_number_next_is_not_1.clone()
                * (ci_next.clone() - opcode_hash.clone())
                * (ci_next.clone() - opcode_absorb.clone())
                * running_evaluation_sponge_has_accumulated_next_row;

        let compressed_difference_of_rows: ConstraintCircuitMonad<_, _> = state_weights[..RATE]
            .iter()
            .zip_eq(difference_of_state_registers[..RATE].iter())
            .map(|(weight, state_difference)| weight.clone() * state_difference.clone())
            .sum();
        let running_evaluation_sponge_absorb_has_accumulated_difference_of_rows =
            running_evaluation_sponge_next.clone()
                - sponge_indeterminate * running_evaluation_sponge.clone()
                - challenge(CIWeight) * ci_next.clone()
                - compressed_difference_of_rows;
        let if_round_no_next_is_1_and_ci_next_is_absorb_then_sponge_absorb_eval_is_updated =
            round_number_next_is_not_1
                * (ci_next.clone() - opcode_hash)
                * (ci_next.clone() - opcode_absorb_init.clone())
                * (ci_next.clone() - opcode_squeeze.clone())
                * running_evaluation_sponge_absorb_has_accumulated_difference_of_rows;

        let running_evaluation_sponge_absorb_remains =
            running_evaluation_sponge_next - running_evaluation_sponge;
        let if_round_no_next_is_not_1_then_running_evaluation_sponge_absorb_remains =
            round_number_next_is_1 * running_evaluation_sponge_absorb_remains.clone();
        let if_ci_next_is_not_spongy_then_running_evaluation_sponge_absorb_remains =
            (ci_next.clone() - opcode_absorb_init)
                * (ci_next.clone() - opcode_absorb)
                * (ci_next - opcode_squeeze)
                * running_evaluation_sponge_absorb_remains;
        let running_evaluation_sponge_absorb_is_updated_correctly =
            if_round_no_next_1_and_ci_next_absorb_init_or_squeeze_then_running_eval_sponge_updates
                + if_round_no_next_is_1_and_ci_next_is_absorb_then_sponge_absorb_eval_is_updated
                + if_round_no_next_is_not_1_then_running_evaluation_sponge_absorb_remains
                + if_ci_next_is_not_spongy_then_running_evaluation_sponge_absorb_remains;

        [
            vec![
                round_number_is_1_through_9_or_round_number_next_is_0,
                round_number_is_0_through_8_or_round_number_next_is_0_or_1,
                round_number_is_0_or_9_or_increments_by_one,
                if_ci_is_hash_then_ci_doesnt_change,
                if_round_number_is_not_9_then_ci_doesnt_change,
            ],
            hash_function_round_correctly_performs_update,
            vec![
                if_round_number_next_is_1_and_ci_next_is_absorb_then_capacity_doesnt_change,
                if_round_number_next_is_1_and_ci_next_is_squeeze_then_state_doesnt_change,
                running_evaluation_hash_input_is_updated_correctly,
                running_evaluation_hash_digest_is_updated_correctly,
                running_evaluation_sponge_absorb_is_updated_correctly,
            ],
        ]
        .concat()
        .into_iter()
        .map(|circuit| circuit.consume())
        .collect()
    }

    pub fn ext_terminal_constraints_as_circuits() -> Vec<
        ConstraintCircuit<
            HashTableChallenges,
            SingleRowIndicator<NUM_BASE_COLUMNS, NUM_EXT_COLUMNS>,
        >,
    > {
        // no more constraints
        vec![]
    }
}

impl HashTable {
    pub fn fill_trace(
        hash_table: &mut ArrayViewMut2<BFieldElement>,
        aet: &AlgebraicExecutionTrace,
    ) {
        let sponge_part_start = 0;
        let sponge_part_end = sponge_part_start + aet.sponge_trace.nrows();
        let hash_part_start = sponge_part_end;
        let hash_part_end = hash_part_start + aet.hash_trace.nrows();

        let sponge_part = hash_table.slice_mut(s![sponge_part_start..sponge_part_end, ..]);
        aet.sponge_trace.clone().move_into(sponge_part);
        let hash_part = hash_table.slice_mut(s![hash_part_start..hash_part_end, ..]);
        aet.hash_trace.clone().move_into(hash_part);
    }

    pub fn pad_trace(hash_table: &mut ArrayViewMut2<BFieldElement>, hash_table_length: usize) {
        hash_table
            .slice_mut(s![hash_table_length.., CI.base_table_index()])
            .fill(Instruction::Hash.opcode_b());
    }

    pub fn extend(
        base_table: ArrayView2<BFieldElement>,
        mut ext_table: ArrayViewMut2<XFieldElement>,
        challenges: &HashTableChallenges,
    ) {
        assert_eq!(BASE_WIDTH, base_table.ncols());
        assert_eq!(EXT_WIDTH, ext_table.ncols());
        assert_eq!(base_table.nrows(), ext_table.nrows());
        let mut hash_input_running_evaluation = EvalArg::default_initial();
        let mut hash_digest_running_evaluation = EvalArg::default_initial();
        let mut sponge_running_evaluation = EvalArg::default_initial();

        let rate_registers = |row: ArrayView1<BFieldElement>| {
            [
                row[STATE0.base_table_index()],
                row[STATE1.base_table_index()],
                row[STATE2.base_table_index()],
                row[STATE3.base_table_index()],
                row[STATE4.base_table_index()],
                row[STATE5.base_table_index()],
                row[STATE6.base_table_index()],
                row[STATE7.base_table_index()],
                row[STATE8.base_table_index()],
                row[STATE9.base_table_index()],
            ]
        };
        let state_weights = [
            challenges.hash_state_weight0,
            challenges.hash_state_weight1,
            challenges.hash_state_weight2,
            challenges.hash_state_weight3,
            challenges.hash_state_weight4,
            challenges.hash_state_weight5,
            challenges.hash_state_weight6,
            challenges.hash_state_weight7,
            challenges.hash_state_weight8,
            challenges.hash_state_weight9,
        ];

        let opcode_hash = Instruction::Hash.opcode_b();
        let opcode_absorb_init = Instruction::AbsorbInit.opcode_b();
        let opcode_absorb = Instruction::Absorb.opcode_b();
        let opcode_squeeze = Instruction::Squeeze.opcode_b();

        let previous_row = Array1::zeros([BASE_WIDTH]);
        let mut previous_row = previous_row.view();
        for row_idx in 0..base_table.nrows() {
            let current_row = base_table.row(row_idx);
            let current_instruction = current_row[CI.base_table_index()];

            if current_row[ROUNDNUMBER.base_table_index()].value() == NUM_ROUNDS as u64 + 1
                && current_instruction == opcode_hash
            {
                // add compressed digest to running evaluation “hash digest”
                let compressed_hash_digest: XFieldElement = rate_registers(current_row)
                    [..DIGEST_LENGTH]
                    .iter()
                    .zip_eq(state_weights[..DIGEST_LENGTH].iter())
                    .map(|(&state, &weight)| weight * state)
                    .sum();
                hash_digest_running_evaluation = hash_digest_running_evaluation
                    * challenges.hash_digest_eval_indeterminate
                    + compressed_hash_digest;
            }

            // all remaining Evaluation Arguments only get updated if the round number is 1
            if current_row[ROUNDNUMBER.base_table_index()].is_one() {
                let elements_for_hash_input_and_sponge_operations = match current_instruction {
                    op if op == opcode_hash || op == opcode_absorb_init || op == opcode_squeeze => {
                        rate_registers(current_row)
                    }
                    op if op == opcode_absorb => {
                        let rate_previous_row = rate_registers(previous_row);
                        let rate_current_row = rate_registers(current_row);
                        rate_current_row
                            .iter()
                            .zip_eq(rate_previous_row.iter())
                            .map(|(&current_state, &previous_state)| current_state - previous_state)
                            .collect_vec()
                            .try_into()
                            .unwrap()
                    }
                    _ => panic!("Opcode must be of `hash`, `absorb_init`, `absorb`, or `squeeze`."),
                };
                let compressed_row_hash_input_and_sponge_operations: XFieldElement = state_weights
                    .iter()
                    .zip_eq(elements_for_hash_input_and_sponge_operations.iter())
                    .map(|(&weight, &element)| weight * element)
                    .sum();

                match current_instruction {
                    ci if ci == opcode_hash => {
                        hash_input_running_evaluation = hash_input_running_evaluation
                            * challenges.hash_input_eval_indeterminate
                            + compressed_row_hash_input_and_sponge_operations;
                    }
                    ci if ci == opcode_absorb_init
                        || ci == opcode_absorb
                        || ci == opcode_squeeze =>
                    {
                        sponge_running_evaluation = sponge_running_evaluation
                            * challenges.sponge_eval_indeterminate
                            + challenges.ci_weight * ci
                            + compressed_row_hash_input_and_sponge_operations;
                    }
                    _ => panic!("Opcode must be of `hash`, `absorb_init`, `absorb`, or `squeeze`."),
                }
            }

            let mut extension_row = ext_table.row_mut(row_idx);
            extension_row[HashInputRunningEvaluation.ext_table_index()] =
                hash_input_running_evaluation;
            extension_row[HashDigestRunningEvaluation.ext_table_index()] =
                hash_digest_running_evaluation;
            extension_row[SpongeRunningEvaluation.ext_table_index()] = sponge_running_evaluation;

            previous_row = current_row;
        }
    }
}

#[derive(Debug, Copy, Clone, Display, EnumCountMacro, EnumIter, PartialEq, Eq, Hash)]
pub enum HashTableChallengeId {
    HashInputEvalIndeterminate,
    HashDigestEvalIndeterminate,
    SpongeEvalIndeterminate,

    CIWeight,
    HashStateWeight0,
    HashStateWeight1,
    HashStateWeight2,
    HashStateWeight3,
    HashStateWeight4,
    HashStateWeight5,
    HashStateWeight6,
    HashStateWeight7,
    HashStateWeight8,
    HashStateWeight9,
    HashStateWeight10,
    HashStateWeight11,
    HashStateWeight12,
    HashStateWeight13,
    HashStateWeight14,
    HashStateWeight15,
}

impl From<HashTableChallengeId> for usize {
    fn from(val: HashTableChallengeId) -> Self {
        val as usize
    }
}

#[derive(Debug, Clone)]
pub struct HashTableChallenges {
    /// The weight that combines two consecutive rows in the
    /// permutation/evaluation column of the hash table.
    pub hash_input_eval_indeterminate: XFieldElement,
    pub hash_digest_eval_indeterminate: XFieldElement,
    pub sponge_eval_indeterminate: XFieldElement,

    /// Weights for condensing part of a row into a single column. (Related to processor table.)
    pub ci_weight: XFieldElement,
    pub hash_state_weight0: XFieldElement,
    pub hash_state_weight1: XFieldElement,
    pub hash_state_weight2: XFieldElement,
    pub hash_state_weight3: XFieldElement,
    pub hash_state_weight4: XFieldElement,
    pub hash_state_weight5: XFieldElement,
    pub hash_state_weight6: XFieldElement,
    pub hash_state_weight7: XFieldElement,
    pub hash_state_weight8: XFieldElement,
    pub hash_state_weight9: XFieldElement,
    pub hash_state_weight10: XFieldElement,
    pub hash_state_weight11: XFieldElement,
    pub hash_state_weight12: XFieldElement,
    pub hash_state_weight13: XFieldElement,
    pub hash_state_weight14: XFieldElement,
    pub hash_state_weight15: XFieldElement,
}

impl TableChallenges for HashTableChallenges {
    type Id = HashTableChallengeId;

    #[inline]
    fn get_challenge(&self, id: Self::Id) -> XFieldElement {
        match id {
            HashInputEvalIndeterminate => self.hash_input_eval_indeterminate,
            HashDigestEvalIndeterminate => self.hash_digest_eval_indeterminate,
            SpongeEvalIndeterminate => self.sponge_eval_indeterminate,
            CIWeight => self.ci_weight,
            HashStateWeight0 => self.hash_state_weight0,
            HashStateWeight1 => self.hash_state_weight1,
            HashStateWeight2 => self.hash_state_weight2,
            HashStateWeight3 => self.hash_state_weight3,
            HashStateWeight4 => self.hash_state_weight4,
            HashStateWeight5 => self.hash_state_weight5,
            HashStateWeight6 => self.hash_state_weight6,
            HashStateWeight7 => self.hash_state_weight7,
            HashStateWeight8 => self.hash_state_weight8,
            HashStateWeight9 => self.hash_state_weight9,
            HashStateWeight10 => self.hash_state_weight10,
            HashStateWeight11 => self.hash_state_weight11,
            HashStateWeight12 => self.hash_state_weight12,
            HashStateWeight13 => self.hash_state_weight13,
            HashStateWeight14 => self.hash_state_weight14,
            HashStateWeight15 => self.hash_state_weight15,
        }
    }
}

#[cfg(test)]
mod constraint_tests {
    use num_traits::Zero;

    use crate::stark::triton_stark_tests::parse_simulate_pad_extend;
    use crate::table::extension_table::Evaluable;
    use crate::table::master_table::MasterTable;

    use super::*;

    #[test]
    fn hash_table_satisfies_constraints_test() {
        let source_code = "hash hash hash halt";
        let (_, _, master_base_table, master_ext_table, challenges) =
            parse_simulate_pad_extend(source_code, vec![], vec![]);
        assert_eq!(
            master_base_table.master_base_matrix.nrows(),
            master_ext_table.master_ext_matrix.nrows()
        );
        let master_base_trace_table = master_base_table.trace_table();
        let master_ext_trace_table = master_ext_table.trace_table();
        assert_eq!(
            master_base_trace_table.nrows(),
            master_ext_trace_table.nrows()
        );

        let num_rows = master_base_trace_table.nrows();
        let first_base_row = master_base_trace_table.row(0);
        let first_ext_row = master_ext_trace_table.row(0);
        for (idx, v) in
            ExtHashTable::evaluate_initial_constraints(first_base_row, first_ext_row, &challenges)
                .iter()
                .enumerate()
        {
            assert!(v.is_zero(), "Initial constraint {idx} failed.");
        }

        for row_idx in 0..num_rows {
            let base_row = master_base_trace_table.row(row_idx);
            let ext_row = master_ext_trace_table.row(row_idx);
            for (constraint_idx, v) in
                ExtHashTable::evaluate_consistency_constraints(base_row, ext_row, &challenges)
                    .iter()
                    .enumerate()
            {
                assert!(
                    v.is_zero(),
                    "consistency constraint {constraint_idx} failed in row {row_idx}"
                );
            }
        }

        for row_idx in 0..num_rows - 1 {
            let base_row = master_base_trace_table.row(row_idx);
            let ext_row = master_ext_trace_table.row(row_idx);
            let next_base_row = master_base_trace_table.row(row_idx + 1);
            let next_ext_row = master_ext_trace_table.row(row_idx + 1);
            for (constraint_idx, v) in ExtHashTable::evaluate_transition_constraints(
                base_row,
                ext_row,
                next_base_row,
                next_ext_row,
                &challenges,
            )
            .iter()
            .enumerate()
            {
                assert!(
                    v.is_zero(),
                    "transition constraint {constraint_idx} failed in row {row_idx}",
                );
            }
        }

        let last_base_row = master_base_trace_table.row(num_rows - 1);
        let last_ext_row = master_ext_trace_table.row(num_rows - 1);
        for (idx, v) in
            ExtHashTable::evaluate_terminal_constraints(last_base_row, last_ext_row, &challenges)
                .iter()
                .enumerate()
        {
            assert!(v.is_zero(), "Terminal constraint {idx} failed.");
        }
    }
}
