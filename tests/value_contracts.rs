use helium::{
    frontend::static_type::{IntegerType as I, StaticType as T, UnsignedIntegerType as U},
    values::*,
};

#[test]
fn existing_values_resolve_copy_through_program_contracts() {
    let program = helium::analyze("fn use_text(s: &str) { var a = s; var b = s; } setup { var n:u8 = 1; var m = n; var k = n; } loop {}").unwrap();
    for (id, symbol) in program.symbols().iter() {
        if matches!(
            symbol.kind,
            helium::semantic::SymbolKind::Local | helium::semantic::SymbolKind::Parameter
        ) {
            assert_eq!(program.value_contracts().transfer(id), Some(Transfer::Copy));
            let ty = program.value_contracts().type_of(id).unwrap();
            assert_eq!(
                program
                    .value_contracts()
                    .registry()
                    .implementation(ty, Capability::Clone),
                Some(&Implementation::Bitwise)
            );
        }
    }
}

fn byte(registry: &mut Registry) -> TypeId {
    registry.source_type(T::Integer(I::Unsigned(U::Uns8)))
}

#[test]
fn user_copy_marker_changes_transfer_without_changing_ownership_rules() {
    let mut registry = Registry::default();
    let byte = byte(&mut registry);
    let record = registry.record(vec![byte]).unwrap();
    assert_eq!(registry.transfer(record), Ok(Transfer::Move));
    // A future frontend derive/marker uses these exact checked registrations.
    registry
        .implement(record, Capability::Clone, Implementation::Structural)
        .unwrap();
    registry
        .implement(record, Capability::Copy, Implementation::Bitwise)
        .unwrap();
    assert_eq!(registry.transfer(record), Ok(Transfer::Copy));
    assert_eq!(
        registry.implement(record, Capability::Copy, Implementation::Bitwise),
        Err(Error::Duplicate(Capability::Copy))
    );
}

#[test]
fn arrays_clone_structurally_but_never_copy_implicitly() {
    let mut registry = Registry::default();
    let byte = byte(&mut registry);
    let array = registry.array(byte).unwrap();
    registry
        .implement(array, Capability::Clone, Implementation::Structural)
        .unwrap();
    assert_eq!(registry.transfer(array), Ok(Transfer::Move));
    assert_eq!(
        registry.implement(array, Capability::Copy, Implementation::Bitwise),
        Err(Error::Forbidden(Capability::Copy))
    );
    let wrapper = registry.record(vec![array]).unwrap();
    registry
        .implement(wrapper, Capability::Clone, Implementation::Structural)
        .unwrap();
    assert_eq!(
        registry.implement(wrapper, Capability::Copy, Implementation::Bitwise),
        Err(Error::MissingFieldCapability {
            field: array,
            capability: Capability::Copy
        })
    );
    assert_eq!(registry.transfer(wrapper), Ok(Transfer::Move));
}

#[test]
fn clone_functions_have_checked_signatures_and_nominal_identity() {
    let mut registry = Registry::default();
    let byte = byte(&mut registry);
    let record = registry.record(vec![byte]).unwrap();
    let valid = Method {
        implementation: ImplementationId(17),
        receiver: Receiver::Shared,
        parameters: vec![],
        result: record,
    };
    for bad in [
        Method {
            receiver: Receiver::Value,
            ..valid.clone()
        },
        Method {
            receiver: Receiver::Exclusive,
            ..valid.clone()
        },
        Method {
            result: byte,
            ..valid.clone()
        },
        Method {
            parameters: vec![byte],
            ..valid.clone()
        },
    ] {
        assert_eq!(
            registry.implement(record, Capability::Clone, Implementation::Function(bad)),
            Err(Error::InvalidSignature)
        );
        assert!(registry.implementation(record, Capability::Clone).is_none());
    }
    registry
        .implement(
            record,
            Capability::Clone,
            Implementation::Function(valid.clone()),
        )
        .unwrap();
    assert_eq!(
        registry.implementation(record, Capability::Clone),
        Some(&Implementation::Function(valid))
    );
    assert_eq!(
        registry.implement(record, Capability::Clone, Implementation::Structural),
        Err(Error::Duplicate(Capability::Clone))
    );
}

#[test]
fn hardware_and_exclusive_access_cannot_be_hidden_inside_cloneable_containers() {
    let mut registry = Registry::default();
    let hardware = registry.hardware();
    let exclusive = registry.exclusive_reference();
    for resource in [hardware, exclusive] {
        let array = registry.array(resource).unwrap();
        let record = registry.record(vec![array]).unwrap();
        for ty in [resource, array, record] {
            let method = Method {
                implementation: ImplementationId(1),
                receiver: Receiver::Shared,
                parameters: vec![],
                result: ty,
            };
            assert_eq!(
                registry.implement(ty, Capability::Clone, Implementation::Function(method)),
                Err(Error::Forbidden(Capability::Clone))
            );
            assert_eq!(
                registry.implement(ty, Capability::Copy, Implementation::Bitwise),
                Err(Error::Forbidden(Capability::Copy))
            );
        }
    }
}

#[test]
fn copy_is_not_user_code_and_generated_clone_requires_cloneable_fields() {
    let mut registry = Registry::default();
    let field = registry.record(vec![]).unwrap();
    let record = registry.record(vec![field]).unwrap();
    assert_eq!(
        registry.implement(record, Capability::Clone, Implementation::Structural),
        Err(Error::MissingFieldCapability {
            field,
            capability: Capability::Clone
        })
    );
    registry
        .implement(field, Capability::Clone, Implementation::Structural)
        .unwrap();
    registry
        .implement(record, Capability::Clone, Implementation::Structural)
        .unwrap();
    let method = Method {
        implementation: ImplementationId(2),
        receiver: Receiver::Shared,
        parameters: vec![],
        result: record,
    };
    assert_eq!(
        registry.implement(record, Capability::Copy, Implementation::Function(method)),
        Err(Error::InvalidImplementation)
    );
}
