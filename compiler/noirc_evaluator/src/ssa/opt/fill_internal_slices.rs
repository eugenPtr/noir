use std::{collections::BTreeMap};

use crate::ssa::{
    ir::{
        basic_block::BasicBlockId,
        function::Function,
        function_inserter::FunctionInserter,
        instruction::{Instruction, Intrinsic},
        post_order::PostOrder,
        types::Type,
        value::{Value, ValueId}, dfg::InsertInstructionResult,
    },
    ssa_gen::Ssa,
};

use acvm::FieldElement;
use fxhash::FxHashMap as HashMap;

impl Ssa {
    /// Fill out slice values represented by SSA array values to contain
    /// dummy data that accounts for dynamic array operations in ACIR gen.
    /// When working with nested slices in ACIR gen it is impossible to discern the size
    /// of internal slices. Thus, we should use the max size of internal nested slices for reading from memory.
    /// However, not increasing the capacity of smaller nested slices can lead to errors where
    /// we end up reading past the specified dynamic index. If we want to also read information that goes
    /// past the nested slice, we will have garbage data in its place if the nested structure is transformed to match.
    pub(crate) fn fill_internal_slices(mut self) -> Ssa {
        for function in self.functions.values_mut() {
            let mut context = Context::new(function);
            context.process_blocks();
        }
        self
    }
}

struct Context<'f> {
    post_order: PostOrder,
    inserter: FunctionInserter<'f>,
}

impl<'f> Context<'f> {
    fn new(function: &'f mut Function) -> Self {
        let post_order = PostOrder::with_function(function);
        let inserter = FunctionInserter::new(function);

        Context { post_order, inserter }
    }

    fn process_blocks(&mut self) {
        let mut block_order = PostOrder::with_function(self.inserter.function).into_vec();
        block_order.reverse();
        for block in block_order {
            self.process_block(block);
        }
    }

    fn process_block(&mut self, block: BasicBlockId) {
        // Fetch SSA values potentially with internal slices
        let instructions = self.inserter.function.dfg[block].take_instructions();
        let mut slice_values = Vec::new();
        // Maps SSA array ID representing slice contents to its length and a list of its potential internal slices
        let mut slice_sizes: HashMap<ValueId, (usize, Vec<ValueId>)> = HashMap::default();
        // let mut new_sizes: HashMap<ValueId, (usize, &[ValueId])> = HashMap::default();
        let mut mapped_slice_values = BTreeMap::new();
        for instruction in instructions.iter() {
            match &self.inserter.function.dfg[*instruction] {
                Instruction::ArrayGet { array, .. } => {
                    let array_typ = self.inserter.function.dfg.type_of_value(*array);
                    let array_value = &self.inserter.function.dfg[*array];
                    if matches!(array_value, Value::Array { .. }) && array_typ.contains_slice_element() {
                        slice_values.push(*array);
                        self.compute_slice_sizes(*array, &mut slice_sizes);
                    }

                    let results = self.inserter.function.dfg.instruction_results(*instruction);
                    let res_typ = self.inserter.function.dfg.type_of_value(results[0]);
                    if res_typ.contains_slice_element() {
                        if let Some(inner_sizes) = slice_sizes.get_mut(array) {
                            inner_sizes.1.push(results[0]);
                            mapped_slice_values.insert(*array, results[0]);

                            let inner_sizes_iter = inner_sizes.1.clone();
                            for slice_value in inner_sizes_iter {
                                let inner_slice = slice_sizes
                                    .get(&slice_value)
                                    .unwrap_or_else(|| panic!("ICE: should have inner slice set for {slice_value}"));
                                slice_sizes.insert(results[0], inner_slice.clone());
    
                                // mapped_slice_values.insert(results[0], slice_value);
                            }
                        }
                    }
                }
                Instruction::ArraySet { array, value, .. } => {
                    let array_typ = self.inserter.function.dfg.type_of_value(*array);
                    let array_value = &self.inserter.function.dfg[*array];
                    if matches!(array_value, Value::Array { .. }) && array_typ.contains_slice_element() {
                        slice_values.push(*array);
                        self.compute_slice_sizes(*array, &mut slice_sizes);
                    }

                    let value_typ = self.inserter.function.dfg.type_of_value(*value);
                    // let value_ssa_value = &self.inserter.function.dfg[*array];
                    if value_typ.contains_slice_element() {
                        self.compute_slice_sizes(*value, &mut slice_sizes);
                    }

                    let results = self.inserter.function.dfg.instruction_results(*instruction);

                    let value_typ = self.inserter.function.dfg.type_of_value(*value);
                    if value_typ.contains_slice_element() {
                        let inner_sizes =
                            slice_sizes.get_mut(array).expect("ICE expected slice sizes");
                        inner_sizes.1.push(*value);

                        mapped_slice_values.insert(*value, results[0]);
                    }

                    if let Some(inner_sizes) = slice_sizes.get_mut(array) {
                        // let inner_sizes =
                        // slice_sizes.get_mut(array).expect(&format!("ICE expected slice sizes for {array}"));

                        let inner_sizes = inner_sizes.clone();
                        slice_sizes.insert(results[0], inner_sizes);

                        // TODO: probably should include checks with array get as well
                        mapped_slice_values.insert(*array, results[0]);       
                    } else {
                        dbg!("got here");
                    }
                }
                Instruction::Call { func, arguments } => {
                    let results = self.inserter.function.dfg.instruction_results(*instruction);

                    let func = &self.inserter.function.dfg[*func];
                    match func {
                        Value::Intrinsic(intrinsic) => {
                            let (argument_index, result_index) = match intrinsic {
                                Intrinsic::SlicePushBack | Intrinsic::SlicePushFront | Intrinsic::SlicePopBack | Intrinsic::SliceInsert | Intrinsic::SliceRemove => {
                                    (1, 1)
                                }
                                Intrinsic::SlicePopFront => {
                                    (1, 2)
                                }
                                _ => continue,
                            };
                            match intrinsic {
                                Intrinsic::SlicePushBack
                                | Intrinsic::SlicePushFront
                                | Intrinsic::SliceInsert => {
                                    let slice_contents = arguments[argument_index];
                                    if let Some(inner_sizes) = slice_sizes.get_mut(&slice_contents) {
                                        // dbg!(inner_sizes.clone());
                                        inner_sizes.0 += 1;
    
                                        let inner_sizes = inner_sizes.clone();
                                        slice_sizes.insert(results[result_index], inner_sizes);
                
                                        mapped_slice_values.insert(slice_contents, results[result_index]);       
                                    }
                                }
                                Intrinsic::SlicePopBack
                                | Intrinsic::SlicePopFront
                                | Intrinsic::SliceRemove => {
                                    let slice_contents = arguments[argument_index];
                                    // We do not decrement the size on intrinsics that could remove values from a slice.
                                    // This is because we could potentially go back to the smaller slice and not fill in dummies.
                                    // This pass should be tracking the potential max that a slice ***could be***
                                    if let Some(inner_sizes) = slice_sizes.get(&slice_contents) {    
                                        let inner_sizes = inner_sizes.clone();
                                        slice_sizes.insert(results[result_index], inner_sizes);
                
                                        mapped_slice_values.insert(slice_contents, results[result_index]);       
                                    }
                                }
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                }
                
                _ => {}
            }
        }

        // Fetch the nested slice max
        let mut nested_slice_max = 0;
        for slice_value in &slice_values {
            let mut mapped_slice_value = *slice_value;
            Self::follow_mapped_slice_values(
                *slice_value,
                &mapped_slice_values,
                &mut mapped_slice_value,
            );

            let nested_depth = self.find_max_nested_depth(mapped_slice_value, &slice_sizes);
            dbg!(nested_depth);
            if nested_depth > nested_slice_max {
                nested_slice_max = nested_depth;
            }
        }

        for instruction in instructions {
            match &self.inserter.function.dfg[instruction] {
                Instruction::ArrayGet { array, .. } => {
                    let typ = self.inserter.function.dfg.type_of_value(*array);
                    if slice_values.contains(array) {
                        let mut mapped_slice_value = *array;
                        Self::follow_mapped_slice_values(
                            *array,
                            &mapped_slice_values,
                            &mut mapped_slice_value,
                        );
                        let nested_slice_max = self.find_max_nested_depth(mapped_slice_value, &slice_sizes);
                        dbg!(nested_slice_max);
                        let new_array = self.attach_slice_dummies(
                                &typ,
                                Some(*array),
                                nested_slice_max,
                                true
                            );

                        let instruction_id = instruction;

                        let (instruction, call_stack) =
                            self.inserter.map_instruction(instruction_id);
                        let new_array_get = match instruction {
                            Instruction::ArrayGet { index, .. } => {
                                Instruction::ArrayGet { array: self.inserter.resolve(new_array), index: self.inserter.resolve(index) }
                            }
                            _ => panic!("Expected array get"),
                        };
                        self.inserter.push_instruction_value(
                            new_array_get,
                            instruction_id,
                            block,
                            call_stack,
                        );
                    } else {
                        self.inserter.push_instruction(instruction, block);
                    }
                }
                Instruction::ArraySet { array, .. } => {
                    let typ = self.inserter.function.dfg.type_of_value(*array);

                    if slice_values.contains(array) {
                        let mut mapped_slice_value = *array;
                        Self::follow_mapped_slice_values(
                            *array,
                            &mapped_slice_values,
                            &mut mapped_slice_value,
                        );
                        let nested_slice_max = self.find_max_nested_depth(mapped_slice_value, &slice_sizes);
                        dbg!(nested_slice_max);
                        let new_array = self.attach_slice_dummies(
                            &typ,
                            Some(*array),
                            nested_slice_max,
                            true,
                        );

                        let instruction_id = instruction;
                        let (instruction, call_stack) =
                            self.inserter.map_instruction(instruction_id);
                        let new_array_set = match instruction {
                            Instruction::ArraySet { index, value, .. } => {
                                Instruction::ArraySet { array: self.inserter.resolve(new_array), index, value }
                            }
                            _ => panic!("Expected array set"),
                        };
                        self.inserter.push_instruction_value(
                            new_array_set,
                            instruction_id,
                            block,
                            call_stack,
                        );
                    } else {
                        self.inserter.push_instruction(instruction, block);
                    }
                }
                
                _ => {
                    self.inserter.push_instruction(instruction, block);
                }
            }
        }

        self.inserter.map_terminator_in_place(block);
    }

    fn attach_slice_dummies(
        &mut self,
        typ: &Type,
        value: Option<ValueId>,
        nested_slice_max: usize,
        is_parent_slice: bool,
    ) -> ValueId {
        match typ {
            Type::Numeric(_) => {
                if let Some(value) = value {
                    self.inserter.resolve(value)
                } else {
                    let zero = FieldElement::zero();
                    self.inserter.function.dfg.make_constant(zero, Type::field())
                }
            }
            Type::Array(element_types, len) => {
                if let Some(value) = value {
                    self.inserter.resolve(value)
                } else {
                    let mut array = im::Vector::new();
                    for _ in 0..*len {
                        for typ in element_types.iter() {
                            array.push_back(self.attach_slice_dummies(typ, None, nested_slice_max, false));
                        }
                    }
                    self.inserter.function.dfg.make_array(array, typ.clone())
                }
            }
            Type::Slice(element_types) => {
                // TODO: Optimize this max to use the nested slice max that follows the type structure
                let mut max_size = nested_slice_max;
                if let Some(value) = value {
                    let mut slice = im::Vector::new();
                    match &self.inserter.function.dfg[value].clone() {
                        Value::Array { array, .. } => {
                            if is_parent_slice {
                                max_size = array.len() / element_types.len();
                            }
                            for i in 0..max_size {
                                for (element_index, element_type) in
                                    element_types.iter().enumerate()
                                {
                                    let index_usize = i * element_types.len() + element_index;
                                    if index_usize < array.len() {
                                        slice.push_back(self.attach_slice_dummies(
                                            element_type,
                                            Some(array[index_usize]),
                                            nested_slice_max,
                                            false,
                                        ));
                                    } else {
                                        slice.push_back(self.attach_slice_dummies(
                                            element_type,
                                            None,
                                            nested_slice_max,
                                            false
                                        ));
                                    }
                                }
                            }
                        }
                        _ => {
                            panic!("Expected an array value");
                        }
                    }
                    self.inserter.function.dfg.make_array(slice, typ.clone())
                } else {
                    let mut slice = im::Vector::new();
                    for _ in 0..max_size {
                        for typ in element_types.iter() {
                            slice.push_back(self.attach_slice_dummies(typ, None, nested_slice_max, false));
                        }
                    }
                    self.inserter.function.dfg.make_array(slice, typ.clone())
                }
            }
            Type::Reference => {
                unreachable!("ICE: Generating dummy data for references is unsupported")
            }
            Type::Function => {
                unreachable!("ICE: Generating dummy data for functions is unsupported")
            }
        }
    }

    fn compute_slice_sizes(
        &self,
        array_id: ValueId,
        slice_sizes: &mut HashMap<ValueId, (usize, Vec<ValueId>)>,
    ) {
        if let Value::Array { array, typ } = &self.inserter.function.dfg[array_id].clone() {
            if let Type::Slice(_) = typ {
                let element_size = typ.element_size();
                let len = array.len() / element_size;
                let mut slice_value = (len, vec![]);
                for value in array {
                    let typ = self.inserter.function.dfg.type_of_value(*value);
                    if let Type::Slice(_) = typ {
                        slice_value.1.push(*value);
                        self.compute_slice_sizes(*value, slice_sizes);
                    }
                }
                // Mark the correct max size based upon an array values internal structure
                let mut max_size = 0;
                for inner_value in slice_value.1.iter() {
                    let inner_slice =
                        slice_sizes.get(inner_value).expect("ICE: should have inner slice set");
                    if inner_slice.0 > max_size {
                        max_size = inner_slice.0;
                    }
                }
                for inner_value in slice_value.1.iter() {
                    let inner_slice =
                        slice_sizes.get_mut(inner_value).expect("ICE: should have inner slice set");
                    if inner_slice.0 < max_size {
                        inner_slice.0 = max_size;
                    }
                }
                slice_sizes.insert(array_id, slice_value);
            }
        }
    }

    fn find_max_nested_depth(
        &self,
        array_id: ValueId,
        slice_sizes: &HashMap<ValueId, (usize, Vec<ValueId>)>,
    ) -> usize {
        let (current_size, inner_slices) = slice_sizes
            .get(&array_id)
            .unwrap_or_else(|| panic!("should have slice sizes: {array_id}"));
        let mut max = *current_size;
        for inner_slice in inner_slices.iter() {
            if let Some(inner_max) = self.compute_inner_max_size(*inner_slice, slice_sizes) {
                if inner_max > max {
                    max = inner_max;
                }
            }
            let inner_nested_max = self.find_max_nested_depth(*inner_slice, slice_sizes);
            if inner_nested_max > max {
                max = inner_nested_max;
            }
        }
        max
    }

    fn compute_inner_max_size(
        &self,
        current_array_id: ValueId,
        slice_sizes: &HashMap<ValueId, (usize, Vec<ValueId>)>,
    ) -> Option<usize> {
        let (_, inner_slices) =
            slice_sizes.get(&current_array_id).expect("should have slice sizes");
        let mut max_size = None;
        for inner_slice in inner_slices.iter() {
            let (inner_size, _) = slice_sizes.get(inner_slice).expect("should have slice sizes");
            if let Some(inner_max) = max_size {
                if *inner_size > inner_max {
                    max_size = Some(*inner_size);
                }
            } else {
                max_size = Some(*inner_size);
            }
        }
        max_size
    }

    fn follow_mapped_slice_values(
        array_id: ValueId,
        mapped_slice_values: &BTreeMap<ValueId, ValueId>,
        new_array_id: &mut ValueId,
    ) {
        if let Some(value) = mapped_slice_values.get(&array_id) {
            *new_array_id = *value;
            Self::follow_mapped_slice_values(*value, mapped_slice_values, new_array_id);
        }
    }
}