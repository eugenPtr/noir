use acvm::acir::brillig::{
    BinaryFieldOp, BinaryIntOp, Opcode as BrilligOpcode, RegisterIndex, Value,
};

use crate::{brillig::brillig_ir::artifact::GeneratedBrillig, ssa::ir::value::ValueId};

/// Generates brillig bytecode which computes the inverse of its input if not null, and zero else.
pub(crate) fn directive_invert() -> GeneratedBrillig {
    //  We generate the following code:
    // fn invert(x : Field) -> Field {
    //    1/ x
    // }

    // The input argument, ie the value that will be inverted.
    // We store the result in this register too.
    let input = RegisterIndex::from(0);
    let one_const = RegisterIndex::from(1);
    // Location of the stop opcode
    let stop_location = 3;

    GeneratedBrillig {
        byte_code: vec![
            // If the input is zero, then we jump to the stop opcode
            BrilligOpcode::JumpIfNot { condition: input, location: stop_location },
            // Put value one in register (1)
            BrilligOpcode::Const { destination: one_const, value: Value::from(1_usize) },
            // Divide 1 by the input, and set the result of the division into register (0)
            BrilligOpcode::BinaryFieldOp {
                op: BinaryFieldOp::Div,
                lhs: one_const,
                rhs: input,
                destination: input,
            },
            BrilligOpcode::Stop,
        ],
        assert_messages: Default::default(),
        locations: Default::default(),
    }
}

/// Generates brillig bytecode which computes `a / b` and returns the quotient and remainder.
///
/// This is equivalent to the Noir (pseudo)code
///
/// ```ignore
/// fn quotient<T>(a: T, b: T) -> (T,T) {
///    (a/b, a-a/b*b)
/// }
/// ```
pub(crate) fn directive_quotient(bit_size: u32) -> GeneratedBrillig {
    // `a` is (0) (i.e register index 0)
    // `b` is (1)
    GeneratedBrillig {
        byte_code: vec![
            //q = a/b is set into register (2)
            BrilligOpcode::BinaryIntOp {
                op: BinaryIntOp::UnsignedDiv,
                lhs: RegisterIndex::from(0),
                rhs: RegisterIndex::from(1),
                destination: RegisterIndex::from(2),
                bit_size,
            },
            //(1)= q*b
            BrilligOpcode::BinaryIntOp {
                op: BinaryIntOp::Mul,
                lhs: RegisterIndex::from(2),
                rhs: RegisterIndex::from(1),
                destination: RegisterIndex::from(1),
                bit_size,
            },
            //(1) = a-q*b
            BrilligOpcode::BinaryIntOp {
                op: BinaryIntOp::Sub,
                lhs: RegisterIndex::from(0),
                rhs: RegisterIndex::from(1),
                destination: RegisterIndex::from(1),
                bit_size,
            },
            //(0) = q
            BrilligOpcode::Mov {
                destination: RegisterIndex::from(0),
                source: RegisterIndex::from(2),
            },
            BrilligOpcode::Stop,
        ],
        assert_messages: Default::default(),
        locations: Default::default(),
    }
}

pub(crate) fn directive_assert_message(inputs: &[ValueId]) -> GeneratedBrillig {
    // let mut inputs = Vec::new();
    // for i in 0..num_inputs {
    //     inputs.push(RegisterOrMemory())
    // }
    // let inputs = 
    GeneratedBrillig {
        byte_code: vec![
            BrilligOpcode::ForeignCall { 
                function: "resolve_assert_message".to_owned(), 
                destinations: vec![], 
                inputs: vec![],
            },
            BrilligOpcode::Stop,
        ],
        assert_messages: Default::default(),
        locations: Default::default(),
    }
}
