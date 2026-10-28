//! Value contracts, shared by ownership analysis and future capability resolution.
//! Source syntax for user declarations is deliberately separate from this registry.
use crate::frontend::static_type::StaticType;
use std::collections::HashMap;

/// Identity local to one registry; never combine IDs from separate programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Copy,
    Clone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Copy,
    Move,
}

/// Nominal function identity supplied by the semantic resolver, never a method name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImplementationId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receiver {
    Shared,
    Exclusive,
    Value,
}

/// The first capability method contract is `clone(&Self) -> Self`.
/// The resolver must register the actual resolved signature, not the annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Method {
    pub implementation: ImplementationId,
    pub receiver: Receiver,
    pub parameters: Vec<TypeId>,
    pub result: TypeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Implementation {
    /// Compiler-known value copy; no user code runs during implicit copying.
    Bitwise,
    /// Clone fields/elements using their resolved Clone implementations.
    Structural,
    Function(Method),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    UnknownType,
    Duplicate(Capability),
    Forbidden(Capability),
    MissingFieldCapability {
        field: TypeId,
        capability: Capability,
    },
    InvalidImplementation,
    InvalidSignature,
}

#[derive(Debug, Clone)]
enum Shape {
    Scalar,
    Shared,
    Exclusive,
    Array(TypeId),
    Record(Vec<TypeId>),
    Hardware,
    /// Reserved source types without an executable value contract yet.
    Opaque,
}

#[derive(Debug, Clone)]
struct Type {
    shape: Shape,
    // Propagated through owning containers; a custom Clone cannot duplicate hardware.
    restricted: bool,
    implementations: HashMap<Capability, Implementation>,
}

#[derive(Debug, Default)]
pub struct Registry {
    types: Vec<Type>,
    source: HashMap<StaticType, TypeId>,
}

/// Per-program resolved contracts. Downstream passes cannot change the policy
/// after semantic analysis or resolve it by spelling of a source type.
#[derive(Debug)]
pub struct Contracts {
    registry: Registry,
    symbols: HashMap<crate::semantic::SymbolId, TypeId>,
}

impl Contracts {
    /// Freeze resolved type contracts, including future user-defined types.
    /// Every symbol must be mapped; later analyses do not invent missing policy.
    pub fn resolved(
        registry: Registry,
        table: &crate::semantic::SymbolTable,
        symbols: HashMap<crate::semantic::SymbolId, TypeId>,
    ) -> Result<Self, Error> {
        if symbols.len() != table.iter().count() {
            return Err(Error::UnknownType);
        }
        for (id, _) in table.iter() {
            registry.get(*symbols.get(&id).ok_or(Error::UnknownType)?)?;
        }
        Ok(Self { registry, symbols })
    }
    pub(crate) fn collect(table: &crate::semantic::SymbolTable) -> Self {
        let mut registry = Registry::default();
        let symbols = table
            .iter()
            .map(|(id, s)| (id, registry.source_type(s.ty)))
            .collect();
        Self::resolved(registry, table, symbols).unwrap()
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    pub fn type_of(&self, symbol: crate::semantic::SymbolId) -> Option<TypeId> {
        self.symbols.get(&symbol).copied()
    }
    pub fn transfer(&self, symbol: crate::semantic::SymbolId) -> Option<Transfer> {
        self.registry.transfer(self.type_of(symbol)?).ok()
    }
    /// Capability of an addressable source value, including a reference's referent.
    pub fn transfer_type(&self, ty: StaticType) -> Option<Transfer> {
        self.registry.transfer(*self.registry.source.get(&ty)?).ok()
    }
}

impl Registry {
    fn add(&mut self, shape: Shape, restricted: bool) -> TypeId {
        let id = TypeId(self.types.len());
        self.types.push(Type {
            shape,
            restricted,
            implementations: HashMap::new(),
        });
        id
    }

    fn get(&self, ty: TypeId) -> Result<&Type, Error> {
        self.types.get(ty.0).ok_or(Error::UnknownType)
    }

    /// A record is nominal and move-only until capabilities are explicitly registered.
    pub fn record(&mut self, fields: Vec<TypeId>) -> Result<TypeId, Error> {
        let mut restricted = false;
        for field in &fields {
            restricted |= self.get(*field)?.restricted;
        }
        Ok(self.add(Shape::Record(fields), restricted))
    }

    /// Arrays are never implicitly Copy, even when all elements are Copy.
    pub fn array(&mut self, element: TypeId) -> Result<TypeId, Error> {
        let restricted = self.get(element)?.restricted;
        Ok(self.add(Shape::Array(element), restricted))
    }

    pub fn hardware(&mut self) -> TypeId {
        self.add(Shape::Hardware, true)
    }

    pub fn exclusive_reference(&mut self) -> TypeId {
        self.add(Shape::Exclusive, true)
    }

    pub fn shared_reference(&mut self) -> TypeId {
        self.copy_value(Shape::Shared)
    }

    fn copy_value(&mut self, shape: Shape) -> TypeId {
        let id = self.add(shape, false);
        // Bootstrap through the same validation path used by declared types.
        self.implement(id, Capability::Clone, Implementation::Bitwise)
            .unwrap();
        self.implement(id, Capability::Copy, Implementation::Bitwise)
            .unwrap();
        id
    }

    /// Only this adapter knows the legacy frontend type enum. Ownership analysis
    /// asks transfer(TypeId), so future nominal types need no new ownership rules.
    pub fn source_type(&mut self, ty: StaticType) -> TypeId {
        if let Some(id) = self.source.get(&ty) {
            return *id;
        }
        if let Some((referent, _)) = ty.reference() {
            self.source_type(referent);
        }
        let id = match ty {
            StaticType::Integer(_)
            | StaticType::Float(_)
            | StaticType::Boolean
            | StaticType::PinState
            | StaticType::PinMode => self.copy_value(Shape::Scalar),
            StaticType::Text => self.shared_reference(),
            StaticType::FixedArray(element, _) => {
                let element = self.source_type(element.ty());
                let array = self.array(element).unwrap();
                self.implement(array, Capability::Clone, Implementation::Structural)
                    .unwrap();
                array
            }
            StaticType::Ref(_, true)
            | StaticType::ArrayRef(_, _, true)
            | StaticType::SliceRef(_, true) => self.exclusive_reference(),
            StaticType::Ref(_, false)
            | StaticType::ArrayRef(_, _, false)
            | StaticType::SliceRef(_, false) => self.shared_reference(),
            StaticType::Array(element) => {
                let element = self.source_type(*element);
                let array = self.array(element).unwrap();
                if self.implementation(element, Capability::Clone).is_some() {
                    self.implement(array, Capability::Clone, Implementation::Structural)
                        .unwrap();
                }
                array
            }
            StaticType::Pin => self.hardware(),
            _ => self.add(Shape::Opaque, false),
        };
        self.source.insert(ty, id);
        id
    }

    pub fn implementation(&self, ty: TypeId, capability: Capability) -> Option<&Implementation> {
        self.types.get(ty.0)?.implementations.get(&capability)
    }

    pub fn transfer(&self, ty: TypeId) -> Result<Transfer, Error> {
        self.get(ty)?;
        Ok(if self.implementation(ty, Capability::Copy).is_some() {
            Transfer::Copy
        } else {
            Transfer::Move
        })
    }

    /// Failed registrations leave the registry unchanged. Source markers and
    /// generated implementations must use this same entry point.
    pub fn implement(
        &mut self,
        ty: TypeId,
        capability: Capability,
        implementation: Implementation,
    ) -> Result<(), Error> {
        let definition = self.get(ty)?;
        if definition.implementations.contains_key(&capability) {
            return Err(Error::Duplicate(capability));
        }
        if definition.restricted || matches!(definition.shape, Shape::Opaque) {
            return Err(Error::Forbidden(capability));
        }
        let fields = match &definition.shape {
            Shape::Record(fields) => fields.clone(),
            Shape::Array(element) => vec![*element],
            _ => vec![],
        };
        match capability {
            Capability::Copy => {
                if matches!(definition.shape, Shape::Array(_)) {
                    return Err(Error::Forbidden(capability));
                }
                if implementation != Implementation::Bitwise {
                    return Err(Error::InvalidImplementation);
                }
                // Copy implies the explicit Clone operation is also available.
                if !definition.implementations.contains_key(&Capability::Clone) {
                    return Err(Error::MissingFieldCapability {
                        field: ty,
                        capability: Capability::Clone,
                    });
                }
                for field in fields {
                    if self.implementation(field, Capability::Copy).is_none() {
                        return Err(Error::MissingFieldCapability { field, capability });
                    }
                }
            }
            Capability::Clone => match &implementation {
                Implementation::Bitwise => {
                    if !matches!(definition.shape, Shape::Scalar | Shape::Shared) {
                        return Err(Error::InvalidImplementation);
                    }
                }
                Implementation::Structural => {
                    if !matches!(definition.shape, Shape::Record(_) | Shape::Array(_)) {
                        return Err(Error::InvalidImplementation);
                    }
                    for field in fields {
                        if self.implementation(field, Capability::Clone).is_none() {
                            return Err(Error::MissingFieldCapability { field, capability });
                        }
                    }
                }
                Implementation::Function(method) => {
                    if method.receiver != Receiver::Shared
                        || !method.parameters.is_empty()
                        || method.result != ty
                    {
                        return Err(Error::InvalidSignature);
                    }
                }
            },
        }
        self.types[ty.0]
            .implementations
            .insert(capability, implementation);
        Ok(())
    }
}

/// Resolve capability calls against the same frozen contracts used by move analysis.
/// Future user implementations enter here by identity, never by backend name lookup.
pub(crate) fn resolve_calls(
    expression: &mut crate::frontend::expression::Expression,
    contracts: &Contracts,
    errors: &mut Vec<crate::diagnostic::Diagnostic>,
) {
    use crate::frontend::expression::ExpressionType as E;
    expression.for_each_child_mut(|e| resolve_calls(e, contracts, errors));
    if let E::CapabilityCall {
        capability,
        arguments,
        implementation,
    } = &mut expression.expression_type
    {
        fn receiver_type(
            e: &crate::frontend::expression::Expression,
            contracts: &Contracts,
        ) -> Option<TypeId> {
            match &e.expression_type {
                E::Grouping { expression } => receiver_type(expression, contracts),
                E::Identifier(_) => e.symbol.and_then(|id| contracts.type_of(id)),
                E::Deref(_) | E::Index { .. } if e.place_root().is_some() => {
                    contracts.registry.source.get(&e.static_type).copied()
                }
                _ => None,
            }
        }
        *implementation = arguments
            .first()
            .and_then(|a| receiver_type(a, contracts))
            .and_then(|ty| contracts.registry().implementation(ty, *capability))
            .cloned();
        if implementation.is_none() {
            errors.push(crate::diagnostic::Diagnostic::new(
                crate::diagnostic::DiagnosticKind::Array {
                    detail: format!(
                        "Type '{}' does not implement {capability:?}",
                        expression.static_type
                    ),
                },
                Some(expression.source_location),
            ));
        }
    }
}
