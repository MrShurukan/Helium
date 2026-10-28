//! Lexical resolution and cached static device specialization.
use super::*;
use crate::{
    frontend::{
        expression::{Expression, ExpressionType as E},
        statement::{DeviceDeclaration, Parameter},
        static_type::StaticType,
    },
    intrinsics::DeviceKind,
};

#[derive(Clone)]
struct Template {
    module: String,
    extension: Option<String>,
    statement: Statement,
}
#[derive(Clone)]
struct Extension {
    public: bool,
    kind: DeviceKind,
    methods: BTreeMap<String, usize>,
}
pub(super) struct Expander {
    units: BTreeMap<String, Unit>,
    templates: Vec<Template>,
    functions: BTreeMap<(String, String), usize>,
    extensions: BTreeMap<(String, String), Extension>,
    devices: BTreeMap<String, DeviceKind>,
    cache: BTreeMap<(usize, Vec<String>), String>,
    depth: usize,
    instances: Vec<Statement>,
    ghosts: BTreeSet<String>,
}
impl Expander {
    pub(super) fn new(units: BTreeMap<String, Unit>) -> Result<Self> {
        let mut result = Self {
            units,
            templates: vec![],
            functions: BTreeMap::new(),
            extensions: BTreeMap::new(),
            devices: BTreeMap::new(),
            cache: BTreeMap::new(),
            depth: 0,
            instances: vec![],
            ghosts: BTreeSet::new(),
        };
        for unit in result.units.values() {
            let mut names = BTreeMap::new();
            for s in &unit.statements {
                let declarations: Vec<_> = match &s.statement_type {
                    S::FunctionDeclaration {
                        identifier,
                        name_location,
                        ..
                    }
                    | S::VariableDeclaration {
                        identifier,
                        name_location,
                        ..
                    } => vec![(identifier, *name_location)],
                    S::Extension { identifier, .. } => vec![(identifier, s.source_location)],
                    S::DeviceDeclaration(d) => vec![(&d.identifier, d.name_location)],
                    S::TimerDeclaration(t) => vec![(&t.identifier, t.name_location)],
                    S::DefDeclaration(pins) => pins
                        .iter()
                        .map(|p| (&p.identifier, p.name_location))
                        .collect(),
                    _ => vec![],
                };
                for (name, location) in declarations {
                    if let Some(previous) = names.insert(name, location) {
                        return Err(vec![Diagnostic::new(
                            DiagnosticKind::DuplicateName { name: name.clone() },
                            Some(location),
                        )
                        .with_label(previous, LabelKind::PreviousDeclaration)]);
                    }
                }
                match &s.statement_type {
                    S::DeviceDeclaration(d) => {
                        result
                            .devices
                            .insert(d.identifier.clone(), DeviceKind::UartTx);
                    }
                    S::FunctionDeclaration { identifier, .. } => {
                        result.functions.insert(
                            (unit.name.clone(), identifier.clone()),
                            result.templates.len(),
                        );
                        result.templates.push(Template {
                            module: unit.name.clone(),
                            extension: None,
                            statement: s.clone(),
                        });
                    }
                    S::Extension {
                        public,
                        identifier,
                        receiver,
                        methods,
                    } => {
                        let kind = DeviceKind::parse(receiver).ok_or_else(|| {
                            error(
                                format!("Unknown receiver kind '{receiver}'"),
                                s.source_location,
                            )
                        })?;
                        let mut extension = Extension {
                            public: *public,
                            kind,
                            methods: BTreeMap::new(),
                        };
                        for method in methods {
                            let S::FunctionDeclaration {
                                identifier: name, ..
                            } = &method.statement_type
                            else {
                                unreachable!()
                            };
                            if kind.method(name).is_some() {
                                return Err(error(format!("Extension cannot redefine built-in method '{receiver}.{name}'"),method.source_location));
                            }
                            if extension
                                .methods
                                .insert(name.clone(), result.templates.len())
                                .is_some()
                            {
                                return Err(error(
                                    format!("Duplicate extension method '{name}'"),
                                    method.source_location,
                                ));
                            }
                            let mut statement = method.clone();
                            if let S::FunctionDeclaration { parameters, .. } =
                                &mut statement.statement_type
                            {
                                parameters.insert(
                                    0,
                                    Parameter {
                                        device_type: Some(receiver.clone()),
                                        symbol: None,
                                        identifier: "self".into(),
                                        static_type: StaticType::NoValue,
                                        type_location: s.source_location,
                                        source_location: s.source_location,
                                    },
                                );
                            }
                            result.templates.push(Template {
                                module: unit.name.clone(),
                                extension: Some(identifier.clone()),
                                statement,
                            });
                        }
                        result
                            .extensions
                            .insert((unit.name.clone(), identifier.clone()), extension);
                    }
                    _ => {}
                }
            }
        }
        for unit in result.units.values() {
            for import in &unit.imports {
                if let Some(name) = &import.extension {
                    let ext = result
                        .extensions
                        .get(&(import.module.clone(), name.clone()))
                        .ok_or_else(|| {
                            error(
                                format!("Unknown extension '{name}' in '{}'", import.module),
                                import.location,
                            )
                        })?;
                    if !ext.public {
                        return Err(error(
                            format!("Extension '{name}' is private"),
                            import.location,
                        ));
                    }
                }
            }
        }
        Ok(result)
    }
    pub(super) fn expand(mut self) -> Result<(Vec<Statement>, BTreeSet<String>)> {
        let mut root = self.units["$entry"].statements.clone();
        root.retain(|s| {
            !matches!(
                s.statement_type,
                S::FunctionDeclaration { .. } | S::Extension { .. } | S::Import { .. }
            )
        });
        self.body(
            &mut root,
            "$entry",
            None,
            &BTreeMap::new(),
            &BTreeSet::new(),
        )?;
        // Check all declarations, including unused library functions and generic bodies.
        // Abstract devices are used only for source checking, never hardware planning/emission.
        for id in 0..self.templates.len() {
            let t = &self.templates[id];
            let S::FunctionDeclaration { parameters, .. } = &t.statement.statement_type else {
                unreachable!()
            };
            let mut bindings = vec![];
            for (index, p) in parameters.iter().enumerate() {
                if let Some(kind) = &p.device_type {
                    let kind = DeviceKind::parse(kind).ok_or_else(|| {
                        error("Unknown static device parameter kind", p.type_location)
                    })?;
                    let name = format!("$check_device_{id}_{index}");
                    self.devices.insert(name.clone(), kind);
                    self.ghosts.insert(name.clone());
                    root.push(Statement {
                        symbol: None,
                        source_location: p.source_location,
                        statement_type: S::DeviceDeclaration(DeviceDeclaration {
                            identifier: name.clone(),
                            hardware: "UART0".into(),
                            baud: 38400,
                            name_location: p.source_location,
                            hardware_location: p.source_location,
                            baud_location: p.source_location,
                        }),
                    });
                    bindings.push(name);
                }
            }
            self.instantiate(id, bindings)?;
        }
        root.extend(self.instances);
        root.sort_by_key(|s| {
            (
                s.source_location.file,
                s.source_location.line_start,
                s.source_location.start_offset,
            )
        });
        Ok((root, self.ghosts))
    }
    fn instantiate(&mut self, id: usize, devices: Vec<String>) -> Result<String> {
        if let Some(name) = self.cache.get(&(id, devices.clone())) {
            return Ok(name.clone());
        }
        // Device bindings are finite; cap expansion to produce a diagnostic instead of exhausting memory.
        if self.cache.len() >= 4096 {
            return Err(error(
                "Static specialization limit exceeded",
                self.templates[id].statement.source_location,
            ));
        }
        if self.depth >= 64 {
            return Err(error(
                "Static call expansion depth exceeds 64",
                self.templates[id].statement.source_location,
            ));
        }
        self.depth += 1;
        let template = self.templates[id].clone();
        let mut statement = template.statement;
        let S::FunctionDeclaration {
            identifier,
            parameters,
            body,
            ..
        } = &mut statement.statement_type
        else {
            unreachable!()
        };
        let source_name = identifier.clone();
        let name = if devices.is_empty() && template.extension.is_none() {
            if template.module == "$entry" {
                source_name
            } else {
                format!("{}::{source_name}", template.module)
            }
        } else {
            format!(
                "{}{}{}<{}>",
                if template.module == "$entry" {
                    String::new()
                } else {
                    format!("{}::", template.module)
                },
                template
                    .extension
                    .as_ref()
                    .map(|e| format!("{e}::"))
                    .unwrap_or_default(),
                source_name,
                devices.join(",")
            )
        };
        self.cache.insert((id, devices.clone()), name.clone());
        if devices.iter().any(|d| self.ghosts.contains(d)) {
            self.ghosts.insert(name.clone());
        }
        *identifier = name.clone();
        let mut bindings = BTreeMap::new();
        let mut runtime = BTreeSet::new();
        let mut index = 0;
        let mut seen = BTreeMap::new();
        for p in parameters.iter() {
            self.check_visible_name(&template.module, &p.identifier, p.source_location)?;
            if let Some(previous) = seen.insert(&p.identifier, p.source_location) {
                return Err(vec![Diagnostic::new(
                    DiagnosticKind::DuplicateName {
                        name: p.identifier.clone(),
                    },
                    Some(p.source_location),
                )
                .with_label(previous, LabelKind::PreviousDeclaration)]);
            }
            if let Some(kind) = &p.device_type {
                let expected = DeviceKind::parse(kind).ok_or_else(|| {
                    error("Unknown static device parameter kind", p.type_location)
                })?;
                let actual = devices
                    .get(index)
                    .ok_or_else(|| error("Missing static device argument", p.source_location))?;
                if self.devices.get(actual) != Some(&expected) {
                    return Err(error("Incompatible device argument", p.source_location));
                }
                bindings.insert(p.identifier.clone(), actual.clone());
                index += 1;
            } else {
                runtime.insert(p.identifier.clone());
            }
        }
        parameters.retain(|p| p.device_type.is_none());
        self.body(
            body,
            &template.module,
            template.extension.as_deref(),
            &bindings,
            &runtime,
        )?;
        self.instances.push(statement);
        self.depth -= 1;
        Ok(name)
    }
    fn device(
        &self,
        name: &str,
        module: &str,
        bindings: &BTreeMap<String, String>,
        location: SourceLocation,
    ) -> Result<String> {
        if let Some(d) = bindings.get(name) {
            return Ok(d.clone());
        }
        if module == "$entry" && self.devices.contains_key(name) {
            return Ok(name.into());
        }
        Err(error(
            format!("'{name}' is not a static device in this scope"),
            location,
        ))
    }
    fn check_visible_name(&self, module: &str, name: &str, location: SourceLocation) -> Result<()> {
        for statement in &self.units[module].statements {
            let declaration = match &statement.statement_type {
                S::FunctionDeclaration {
                    identifier,
                    name_location,
                    ..
                }
                | S::VariableDeclaration {
                    identifier,
                    name_location,
                    ..
                } if identifier == name => Some(*name_location),
                S::Extension { identifier, .. } if identifier == name => {
                    Some(statement.source_location)
                }
                S::DeviceDeclaration(d) if d.identifier == name => Some(d.name_location),
                S::TimerDeclaration(t) if t.identifier == name => Some(t.name_location),
                S::DefDeclaration(pins) => pins
                    .iter()
                    .find(|p| p.identifier == name)
                    .map(|p| p.name_location),
                _ => None,
            };
            if let Some(previous) = declaration {
                return Err(vec![Diagnostic::new(
                    DiagnosticKind::DuplicateName { name: name.into() },
                    Some(location),
                )
                .with_label(previous, LabelKind::PreviousDeclaration)]);
            }
        }
        Ok(())
    }
    fn method(
        &self,
        module: &str,
        own: Option<&str>,
        kind: DeviceKind,
        name: &str,
        location: SourceLocation,
    ) -> Result<Option<usize>> {
        if let Some(own) = own {
            if let Some(id) = self.extensions[&(module.into(), own.into())]
                .methods
                .get(name)
            {
                return Ok(Some(*id));
            }
        }
        let mut candidates = BTreeSet::new();
        for ((owner, _), ext) in &self.extensions {
            if owner == module && ext.kind == kind {
                if let Some(id) = ext.methods.get(name) {
                    if self.is_public(*id) {
                        candidates.insert(*id);
                    }
                }
            }
        }
        for import in &self.units[module].imports {
            if let Some(ext) = &import.extension {
                let ext = &self.extensions[&(import.module.clone(), ext.clone())];
                if ext.kind == kind {
                    if let Some(id) = ext.methods.get(name) {
                        if self.is_public(*id) {
                            candidates.insert(*id);
                        }
                    }
                }
            }
        }
        if candidates.len() > 1 {
            let mut d=Diagnostic::new(DiagnosticKind::Module{detail:format!("Ambiguous extension method '{name}'; use Extension::{name}(device, ...) to select one")},Some(location));
            for id in candidates {
                d = d.with_label(
                    self.templates[id].statement.source_location,
                    LabelKind::FunctionDeclaration,
                );
            }
            return Err(vec![d]);
        }
        Ok(candidates.first().copied())
    }
    fn is_public(&self, id: usize) -> bool {
        matches!(
            self.templates[id].statement.statement_type,
            S::FunctionDeclaration { public: true, .. }
        )
    }
    fn named(
        &self,
        module: &str,
        own: Option<&str>,
        name: &str,
        location: SourceLocation,
    ) -> Result<Option<usize>> {
        if !name.contains("::") {
            return Ok(self.functions.get(&(module.into(), name.into())).copied());
        }
        let (prefix, method) = name.rsplit_once("::").unwrap();
        let mut extensions = BTreeSet::new();
        if self
            .extensions
            .contains_key(&(module.into(), prefix.into()))
        {
            extensions.insert((module.to_string(), prefix.to_string()));
        }
        for import in &self.units[module].imports {
            if let Some(ext) = &import.extension {
                if prefix == ext || prefix == format!("{}::{ext}", import.module) {
                    extensions.insert((import.module.clone(), ext.clone()));
                }
            } else if prefix == import.module {
                if let Some(id) = self.functions.get(&(prefix.into(), method.into())) {
                    if !self.is_public(*id) {
                        return Err(error(format!("Function '{name}' is private"), location));
                    }
                    return Ok(Some(*id));
                }
            }
        }
        if extensions.len() > 1 {
            return Err(error(
                format!("Ambiguous extension name '{prefix}'; qualify its module"),
                location,
            ));
        }
        if let Some(key) = extensions.first() {
            if let Some(id) = self.extensions[key].methods.get(method) {
                if !self.is_public(*id) && !(key.0 == module && Some(key.1.as_str()) == own) {
                    return Err(error(format!("Method '{name}' is private"), location));
                }
                return Ok(Some(*id));
            }
        }
        Err(error(
            format!("Unknown or unimported qualified callable '{name}'"),
            location,
        ))
    }
    fn expression(
        &mut self,
        e: &mut Expression,
        module: &str,
        own: Option<&str>,
        bindings: &BTreeMap<String, String>,
        locals: &BTreeSet<String>,
    ) -> Result<()> {
        // Children are handled here explicitly so static arguments never become runtime values.
        if let E::Call {
            identifier,
            arguments,
        } = &mut e.expression_type
        {
            if let Some((name, method)) = identifier.split_once('.') {
                if !bindings.contains_key(name) && !self.devices.contains_key(name) {
                    use crate::frontend::expression::TextOperation;
                    let operation = match method {
                        "len" => Some(TextOperation::Len),
                        "byte_or" => Some(TextOperation::ByteOr),
                        _ => None,
                    };
                    if operation.is_some() || method == "clone" {
                        let mut args = vec![Expression {
                            symbol: None,
                            expression_type: E::Identifier(name.into()),
                            source_location: e.source_location.name(name),
                            static_type: StaticType::Unknown,
                        }];
                        args.append(arguments);
                        for a in &mut args {
                            self.expression(a, module, own, bindings, locals)?;
                        }
                        e.expression_type = if let Some(operation) = operation {
                            E::Text {
                                operation,
                                arguments: args,
                            }
                        } else {
                            E::CapabilityCall {
                                capability: crate::values::Capability::Clone,
                                arguments: args,
                                implementation: None,
                            }
                        };
                        return Ok(());
                    }
                }
            }
            let mut receiver = None;
            let target = if let Some((device, method)) = identifier.split_once('.') {
                let d = self.device(device, module, bindings, e.source_location)?;
                let kind = self.devices[&d];
                if kind.method(method).is_some() {
                    *identifier = format!("{d}.{method}");
                    None
                } else {
                    let id = self.method(module, own, kind, method, e.source_location)?;
                    receiver = Some(d.clone());
                    if id.is_none() {
                        return Err(vec![Diagnostic::new(
                            DiagnosticKind::UndefinedName {
                                name: identifier.clone(),
                            },
                            Some(e.source_location.name(identifier)),
                        )]);
                    }
                    id
                }
            } else {
                self.named(module, own, identifier, e.source_location)?
            };
            if let Some(id) = target {
                let template = self.templates[id].clone();
                let S::FunctionDeclaration { parameters, .. } = &template.statement.statement_type
                else {
                    unreachable!()
                };
                if let Some(d) = receiver {
                    arguments.insert(
                        0,
                        Expression {
                            symbol: None,
                            expression_type: E::Identifier(d),
                            source_location: e.source_location,
                            static_type: StaticType::NoValue,
                        },
                    );
                }
                if parameters.iter().all(|p| p.device_type.is_none()) {
                    for argument in arguments.iter_mut() {
                        self.expression(argument, module, own, bindings, locals)?;
                    }
                    *identifier = self.instantiate(id, vec![])?;
                    return Ok(());
                }
                if parameters.len() != arguments.len() {
                    return Err(vec![Diagnostic::new(
                        DiagnosticKind::ArgumentCount {
                            name: identifier.clone(),
                            expected: parameters.len()
                                - usize::from(
                                    template.extension.is_some() && identifier.contains('.'),
                                ),
                            actual: arguments.len()
                                - usize::from(
                                    template.extension.is_some() && identifier.contains('.'),
                                ),
                        },
                        Some(e.source_location),
                    )
                    .with_label(
                        template.statement.source_location,
                        LabelKind::FunctionDeclaration,
                    )]);
                }
                let mut devices = vec![];
                let mut runtime = vec![];
                for (p, mut a) in parameters.iter().zip(std::mem::take(arguments)) {
                    if let Some(kind) = &p.device_type {
                        let expected = DeviceKind::parse(kind)
                            .ok_or_else(|| error("Unknown device kind", p.type_location))?;
                        let E::Identifier(name) = &a.expression_type else {
                            return Err(error("Static device arguments must be named devices or static parameters",a.source_location));
                        };
                        let actual = if self.devices.contains_key(name)
                            && (module == "$entry" || bindings.values().any(|v| v == name))
                        {
                            name.clone()
                        } else {
                            self.device(name, module, bindings, a.source_location)?
                        };
                        if self.devices[&actual] != expected {
                            return Err(error("Incompatible device argument", a.source_location));
                        }
                        devices.push(actual);
                    } else {
                        self.expression(&mut a, module, own, bindings, locals)?;
                        runtime.push(a);
                    }
                }
                *arguments = runtime;
                *identifier = self.instantiate(id, devices)?;
            } else {
                for a in arguments {
                    self.expression(a, module, own, bindings, locals)?;
                }
                if module != "$entry" && !identifier.contains('.') {
                    let kind = if locals.contains(identifier) {
                        DiagnosticKind::NotCallable {
                            name: identifier.clone(),
                        }
                    } else {
                        DiagnosticKind::UndefinedName {
                            name: identifier.clone(),
                        }
                    };
                    return Err(vec![Diagnostic::new(
                        kind,
                        Some(e.source_location.name(identifier)),
                    )]);
                }
            }
            return Ok(());
        }
        match &mut e.expression_type {
            E::Assignment {
                identifier,
                expression,
            } => {
                if bindings.contains_key(identifier) {
                    return Err(error(
                        "Static device parameters cannot be assigned",
                        e.source_location,
                    ));
                }
                if module != "$entry" && !locals.contains(identifier) {
                    return Err(vec![Diagnostic::new(
                        DiagnosticKind::UndefinedName {
                            name: identifier.clone(),
                        },
                        Some(e.source_location.name(identifier)),
                    )]);
                }
                self.expression(expression, module, own, bindings, locals)?;
            }
            E::Identifier(name) => {
                if let Some(d) = bindings.get(name) {
                    *name = d.clone();
                } else if module != "$entry" && !locals.contains(name) {
                    if self.functions.contains_key(&(module.into(), name.clone())) {
                        *name = format!("{module}::{name}");
                    } else {
                        return Err(vec![Diagnostic::new(
                            DiagnosticKind::UndefinedName { name: name.clone() },
                            Some(e.source_location),
                        )]);
                    }
                }
            }
            _ => {
                let mut failure = None;
                e.for_each_child_mut(|child| {
                    if failure.is_none() {
                        failure = self.expression(child, module, own, bindings, locals).err();
                    }
                });
                if let Some(errors) = failure {
                    return Err(errors);
                }
            }
        }
        Ok(())
    }
    fn body(
        &mut self,
        statements: &mut [Statement],
        module: &str,
        own: Option<&str>,
        bindings: &BTreeMap<String, String>,
        outer: &BTreeSet<String>,
    ) -> Result<()> {
        let mut locals = outer.clone();
        for s in statements {
            match &mut s.statement_type {
                S::VariableDeclaration {
                    identifier,
                    name_location,
                    initializer,
                    ..
                } => {
                    // Top-level globals are checked by the ordinary analyzer. Local
                    // bindings must also respect names removed by specialization.
                    let is_global=module=="$entry" && self.units[module].statements.iter().any(|s| matches!(&s.statement_type,S::VariableDeclaration{name_location:global,..} if global==name_location));
                    if !is_global {
                        self.check_visible_name(module, identifier, *name_location)?;
                    }
                    if bindings.contains_key(identifier) {
                        return Err(error(
                            "A local cannot shadow a static device parameter",
                            s.source_location,
                        ));
                    }
                    if let Some(e) = initializer {
                        self.expression(e, module, own, bindings, &locals)?;
                    }
                    locals.insert(identifier.clone());
                }
                S::Expression(e) => self.expression(e, module, own, bindings, &locals)?,
                S::Return(Some(e)) => self.expression(e, module, own, bindings, &locals)?,
                S::SetupDeclaration(body)
                | S::LoopDeclaration(body)
                | S::Block(body)
                | S::NoInterrupt(body) => self.body(body, module, own, bindings, &locals)?,
                S::TimerDeclaration(t) => self.body(&mut t.body, module, own, bindings, &locals)?,
                S::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    self.expression(condition, module, own, bindings, &locals)?;
                    self.body(
                        std::slice::from_mut(then_body),
                        module,
                        own,
                        bindings,
                        &locals,
                    )?;
                    if let Some(b) = else_body {
                        self.body(std::slice::from_mut(b), module, own, bindings, &locals)?;
                    }
                }
                S::While { condition, body } => {
                    self.expression(condition, module, own, bindings, &locals)?;
                    self.body(std::slice::from_mut(body), module, own, bindings, &locals)?;
                }
                S::For {
                    identifier,
                    name_location,
                    from,
                    to,
                    body,
                    ..
                } => {
                    self.check_visible_name(module, identifier, *name_location)?;
                    if bindings.contains_key(identifier) {
                        return Err(error(
                            "A loop variable cannot shadow a static device parameter",
                            *name_location,
                        ));
                    }
                    self.expression(from, module, own, bindings, &locals)?;
                    self.expression(to, module, own, bindings, &locals)?;
                    let mut nested = locals.clone();
                    nested.insert(identifier.clone());
                    self.body(std::slice::from_mut(body), module, own, bindings, &nested)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}
