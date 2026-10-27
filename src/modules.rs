//! Module loading and static device specialization, before ordinary type checking.
//! Transform AST identities, never source text; retain original locations in every instance.
use crate::{
    diagnostic::{
        source::{SourceSet, SourceText},
        Diagnostic, DiagnosticKind, LabelKind,
    },
    frontend::{
        statement::{Statement, StatementType as S},
        utils::SourceLocation,
        Parser,
    },
    semantic::AnalyzedProgram,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

type Result<T> = std::result::Result<T, Vec<Diagnostic>>;
fn error(message: impl Into<String>, location: SourceLocation) -> Vec<Diagnostic> {
    vec![Diagnostic::new(
        DiagnosticKind::Module {
            detail: message.into(),
        },
        Some(location),
    )]
}

/// Filesystem access is explicit. In-memory callers can provide named module texts.
/// The default std source is bundled with this compiler, independent of cwd/install paths.
pub struct Project {
    pub sources: SourceSet,
    pub app_root: Option<PathBuf>,
    pub std_root: Option<PathBuf>,
    pub modules: BTreeMap<String, String>,
    entry_path: Option<PathBuf>,
}
impl Project {
    pub fn memory(name: &str, source: &str) -> Self {
        Self {
            sources: SourceSet {
                files: vec![SourceText {
                    name: name.into(),
                    text: source.into(),
                }],
            },
            app_root: None,
            std_root: None,
            modules: BTreeMap::new(),
            entry_path: None,
        }
    }
    pub fn from_file(path: &Path) -> std::io::Result<Self> {
        let path = path.canonicalize()?;
        let mut project = Self::memory(
            &path.display().to_string(),
            &std::fs::read_to_string(&path)?,
        );
        project.app_root = path.parent().map(Path::to_path_buf);
        project.entry_path = Some(path);
        Ok(project)
    }
    pub fn analyze(&mut self) -> Result<AnalyzedProgram> {
        self.sources.files.truncate(1);
        let result = self.resolve();
        let sources = Arc::new(self.sources.clone());
        let attach = |mut diagnostics: Vec<Diagnostic>| {
            if sources.files.len() > 1 {
                for d in &mut diagnostics {
                    d.sources = Some(sources.clone());
                }
            }
            diagnostics
        };
        let (statements, ghosts) = result.map_err(attach)?;
        let mut program = crate::middle_end::Analyzer::new(&self.sources.files[0].text)
            .analyze(statements)
            .map_err(attach)?;
        let mut retained = vec![];
        for statement in std::mem::take(&mut program.statements) {
            let name = match &statement.statement_type {
                S::FunctionDeclaration { identifier, .. } => Some(identifier),
                S::DeviceDeclaration(d) => Some(&d.identifier),
                _ => None,
            };
            if name.is_some_and(|n| ghosts.contains(n)) {
                program.template_checks.push(statement);
            } else {
                retained.push(statement);
            }
        }
        program.statements = retained;
        program.warnings = attach(program.warnings);
        program.sources = sources;
        Ok(program)
    }
    fn parse(&self, file: usize, library: bool) -> Result<Vec<Statement>> {
        let source = &self.sources.files[file].text;
        Parser::new(source)
            .parse_file(file, library)
            .map_err(|errors| {
                use unicode_segmentation::UnicodeSegmentation;
                let line = source.rsplit('\n').next().unwrap_or("");
                let eof = SourceLocation {
                    file,
                    line_start: source.split('\n').count(),
                    line_end: source.split('\n').count(),
                    start_offset: line.graphemes(true).count(),
                    end_offset: line.graphemes(true).count(),
                };
                let mut diagnostics: Vec<_> = errors.into_iter().map(Diagnostic::from).collect();
                for d in &mut diagnostics {
                    if d.location.is_none() {
                        d.location = Some(eof);
                    }
                }
                diagnostics.dedup_by(|a, b| a.location == b.location && a.message() == b.message());
                diagnostics
            })
    }
    fn module_text(&self, name: &str, location: SourceLocation) -> Result<SourceText> {
        if let Some(text) = self.modules.get(name) {
            return Ok(SourceText {
                name: name.into(),
                text: text.clone(),
            });
        }
        let parts: Vec<_> = name.split("::").collect();
        let root = match parts.first().copied() {
            Some("app") => self.app_root.as_ref(),
            Some("std") => self.std_root.as_ref(),
            _ => {
                return Err(error(
                    "Module paths must start with app:: or std::",
                    location,
                ))
            }
        };
        if parts.len() < 2 {
            return Err(error(
                "Expected a module name after app:: or std::",
                location,
            ));
        }
        if parts[0] == "std" && root.is_none() && name == "std::uart" {
            return Ok(SourceText {
                name: "<std>/uart.he".into(),
                text: include_str!("../lib/std/uart.he").into(),
            });
        }
        let root = root.ok_or_else(|| {
            error(
                format!("Cannot load '{name}': supply an application/module root or module source"),
                location,
            )
        })?;
        let mut path = root.clone();
        for part in &parts[1..] {
            path.push(part);
        }
        path.set_extension("he");
        let canonical = path.canonicalize().map_err(|e| {
            error(
                format!("Cannot load module '{name}' ({}): {e}", path.display()),
                location,
            )
        })?;
        let root = root
            .canonicalize()
            .map_err(|e| error(e.to_string(), location))?;
        if !canonical.starts_with(root) {
            return Err(error("Module path escapes its source root", location));
        }
        if self.entry_path.as_ref() == Some(&canonical) {
            return Err(error(
                "Import cycle reaches the application entry file",
                location,
            ));
        }
        let text =
            std::fs::read_to_string(&canonical).map_err(|e| error(e.to_string(), location))?;
        Ok(SourceText {
            name: canonical.display().to_string(),
            text,
        })
    }
    fn load(
        &mut self,
        name: String,
        file: usize,
        units: &mut BTreeMap<String, Unit>,
        stack: &mut Vec<String>,
    ) -> Result<()> {
        if stack.len() >= 64 {
            return Err(error(
                "Module import depth exceeds 64",
                SourceLocation {
                    file,
                    ..SourceLocation::default()
                },
            ));
        }
        stack.push(name.clone());
        let statements = self.parse(file, file != 0)?;
        let mut imports = vec![];
        for s in &statements {
            if file != 0
                && !matches!(
                    s.statement_type,
                    S::Import { .. } | S::FunctionDeclaration { .. } | S::Extension { .. }
                )
            {
                return Err(error("Library modules may contain only imports, functions and extensions; hardware and runtime storage belong to the application",s.source_location));
            }
            if let S::Import { path, extension } = &s.statement_type {
                let min = if *extension { 3 } else { 2 };
                if path.len() < min || !matches!(path[0].as_str(), "app" | "std") {
                    return Err(error(
                        "Expected app::/std::module, with an extension name for extension imports",
                        s.source_location,
                    ));
                }
                let module = path[..path.len() - usize::from(*extension)].join("::");
                if stack.contains(&module) {
                    return Err(error(
                        format!("Import cycle: {} -> {module}", stack.join(" -> ")),
                        s.source_location,
                    ));
                }
                if !units.contains_key(&module) {
                    let text = self.module_text(&module, s.source_location)?;
                    let index = self.sources.files.len();
                    self.sources.files.push(text);
                    self.load(module.clone(), index, units, stack)?;
                }
                imports.push(Import {
                    module,
                    extension: extension.then(|| path.last().unwrap().clone()),
                    location: s.source_location,
                });
            }
        }
        units.insert(
            name.clone(),
            Unit {
                name,
                statements,
                imports,
            },
        );
        stack.pop();
        Ok(())
    }
    fn resolve(&mut self) -> Result<(Vec<Statement>, BTreeSet<String>)> {
        let mut units = BTreeMap::new();
        self.load("$entry".into(), 0, &mut units, &mut vec![])?;
        Expander::new(units)?.expand()
    }
}

#[derive(Clone)]
struct Import {
    module: String,
    extension: Option<String>,
    location: SourceLocation,
}
struct Unit {
    name: String,
    statements: Vec<Statement>,
    imports: Vec<Import>,
}

mod expand;
use expand::Expander;
