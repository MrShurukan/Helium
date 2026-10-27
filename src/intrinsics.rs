//! Typed built-in callables. Source functions infer effects from these contracts.
use crate::{
    effects::{Contract, IrqRequirement, Reason},
    frontend::static_type::{IntegerType, StaticType, UnsignedIntegerType},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intrinsic {
    UartTryWrite,
    UartWrite,
    UartFlush,
}
impl Intrinsic {
    pub const ALL: [Self; 3] = [Self::UartTryWrite, Self::UartWrite, Self::UartFlush];
    pub fn name(self) -> &'static str {
        match self {
            Self::UartTryWrite => "try_write_byte",
            Self::UartWrite => "write_byte",
            Self::UartFlush => "flush",
        }
    }
    pub fn parameters(self) -> Vec<StaticType> {
        if self == Self::UartFlush {
            vec![]
        } else {
            vec![StaticType::Integer(IntegerType::Unsigned(
                UnsignedIntegerType::Uns8,
            ))]
        }
    }
    pub fn result(self) -> Option<StaticType> {
        (self == Self::UartTryWrite).then_some(StaticType::Boolean)
    }
    pub fn contract(self) -> Contract {
        Contract {
            irq: IrqRequirement::Any,
            irq_reason: None,
            blocking: match self {
                Self::UartTryWrite => None,
                Self::UartWrite => Some(Reason::TransmitterReady),
                Self::UartFlush => Some(Reason::TransmissionComplete),
            },
        }
    }
}

/// Semantic receiver kind, independent of the physical target peripheral.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    UartTx,
}
impl DeviceKind {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "UartTx" => Some(Self::UartTx),
            _ => None,
        }
    }
    pub fn methods(self) -> &'static [Intrinsic] {
        match self {
            Self::UartTx => &Intrinsic::ALL,
        }
    }
    pub fn method(self, name: &str) -> Option<Intrinsic> {
        self.methods().iter().copied().find(|m| m.name() == name)
    }
}
