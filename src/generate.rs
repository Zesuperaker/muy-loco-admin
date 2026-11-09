//! The `admin:models` task, which keeps a file listing the app's models in the admin.

use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use async_trait::async_trait;
use loco_rs::{
    Error, Result,
    app::AppContext,
    task::{Task, TaskInfo, Vars},
};

use crate::{resource::is_sensitive, roles::ROLES_TABLE};

/// The comment in the generated file above which `admin:models` adds models.
pub const MODELS_MARKER: &str = "// admin-models-inject (do not remove this comment)";

const DEFAULT_ENTITIES: &str = "src/models/_entities";
const DEFAULT_OUTPUT: &str = "src/admin.rs";
/// Loco's own tables, such as its session store, are never added.
const LOCO_TABLE_PREFIX: &str = "loco_";

/// `cargo loco task admin:models` adds every `SeaORM` entity of the app to a
/// file listing the models in the admin, `src/admin.rs` by default.
///
/// The first run creates the file, with a `models` function that registers
/// each entity in `src/models/_entities`, and declares its module in
/// `src/lib.rs`. Later runs add only the entities the file does not mention
/// yet, above [`MODELS_MARKER`], so a line you comment out stays out of the
/// admin and a model you configure with [`ModelAdmin`](crate::ModelAdmin)
/// keeps its configuration. The file is formatted with `rustfmt` when it is
/// installed.
///
/// The admin's `admin_roles` table and Loco's own `loco_*` tables are never
/// added. An entity whose required column is hidden, like `users.password`,
/// is added with `can_create(false)`, since records could not be created from
/// the admin; one whose primary key looks sensitive is added commented out,
/// since the admin would refuse to hide it.
///
/// ```rust,ignore
/// // src/app.rs, inside `impl Hooks for App`
/// fn register_tasks(tasks: &mut Tasks) {
///     tasks.register(AdminModels::new());
/// }
///
/// async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
///     let admin = Admin::new(JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true)));
///     Ok(vec![Box::new(crate::admin::models(admin).middlewares_from::<Self>())])
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AdminModels {
    entities: PathBuf,
    output: PathBuf,
}

impl Default for AdminModels {
    fn default() -> Self {
        Self::new()
    }
}

/// An entity module found in the entities directory.
#[derive(Debug, PartialEq, Eq)]
struct Entity {
    /// The module as `mod.rs` names it, raw identifiers included (`r#type`).
    module: String,
    hidden: Hidden,
}

/// The first hidden column of an entity that changes how it is registered.
#[derive(Debug, PartialEq, Eq)]
enum Hidden {
    None,
    /// Not an `Option`, so the admin's create form could not fill it in.
    Required(String),
    /// The admin refuses to hide a primary key.
    PrimaryKey(String),
}

/// What a run changed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Report {
    created: bool,
    added: Vec<String>,
    declared: Option<String>,
    formatted: bool,
    warnings: Vec<String>,
}

impl AdminModels {
    /// Read entities from `src/models/_entities` and write `src/admin.rs`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entities: PathBuf::from(DEFAULT_ENTITIES),
            output: PathBuf::from(DEFAULT_OUTPUT),
        }
    }

    /// The directory `cargo loco db entities` writes to, relative to the
    /// project root and under `src/` (defaults to `src/models/_entities`).
    #[must_use]
    pub fn entities(mut self, dir: impl Into<PathBuf>) -> Self {
        self.entities = dir.into();
        self
    }

    /// The file listing the models, relative to the project root and under
    /// `src/` (defaults to `src/admin.rs`). Its module is declared in
    /// `src/lib.rs` only when it is directly in `src/`.
    #[must_use]
    pub fn output(mut self, file: impl Into<PathBuf>) -> Self {
        self.output = file.into();
        self
    }

    /// Create or update the output file under the project root `root`.
    fn generate(&self, root: &Path) -> Result<Report> {
        let entities_dir = under_src(&self.entities, "entities directory")?;
        let output_parts = under_src(&self.output, "output file")?;
        let module_path = format!("crate::{}", entities_dir.join("::"));
        let alias = entities_dir.last().cloned().unwrap_or_default();
        let output_name = format!("src/{}", output_parts.join("/"));

        let mut report = Report::default();
        let entities = read_entities(
            &root.join("src").join(entities_dir.join("/")),
            &mut report.warnings,
        )?;
        let output = root.join(&output_name);

        if output.exists() {
            let text = fs::read_to_string(&output).map_err(Error::wrap)?;
            let missing: Vec<&Entity> = entities
                .iter()
                .filter(|entity| !mentions(&text, &entity.module))
                .collect();
            if !missing.is_empty() {
                let updated = insert_above_marker(&text, &alias, &missing)
                    .map_err(|problem| Error::string(&problem.message(&output_name)))?;
                fs::write(&output, updated).map_err(Error::wrap)?;
                report.added = missing.iter().map(|e| e.module.clone()).collect();
            }
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent).map_err(Error::wrap)?;
            }
            let all: Vec<&Entity> = entities.iter().collect();
            let text = new_file(&module_path, &alias, &self.entities, &all);
            fs::write(&output, text).map_err(Error::wrap)?;
            report.created = true;
            report.added = entities.iter().map(|e| e.module.clone()).collect();
        }

        if report.created || !report.added.is_empty() {
            report.formatted = rustfmt(root, &output_name);
        }
        if let [module] = output_parts.as_slice() {
            let module = module.strip_suffix(".rs").unwrap_or(module);
            report.declared = declare_module(root, module)?;
        }
        Ok(report)
    }
}

#[async_trait]
impl Task for AdminModels {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "admin:models".to_string(),
            detail: format!(
                "Add each entity in `{}` to `{}`, the models in the admin.\nUsage:\ncargo loco \
                 task admin:models",
                self.entities.display(),
                self.output.display()
            ),
        }
    }

    async fn run(&self, _ctx: &AppContext, _vars: &Vars) -> Result<()> {
        let report = self.generate(Path::new("."))?;
        println!("{}", report.summary(&self.output));
        Ok(())
    }
}

impl Report {
    fn summary(&self, output: &Path) -> String {
        let output = output.display();
        let mut lines: Vec<String> = self
            .warnings
            .iter()
            .map(|w| format!("warning: {w}"))
            .collect();
        let models = match self.added.len() {
            1 => "1 model".to_string(),
            n => format!("{n} models"),
        };
        let names = self.added.join(", ");
        if self.created {
            lines.push(format!("Created `{output}` with {models}: {names}"));
        } else if self.added.is_empty() {
            lines.push(format!("`{output}` already lists every entity"));
        } else {
            lines.push(format!("Added {models} to `{output}`: {names}"));
        }
        if (self.created || !self.added.is_empty()) && !self.formatted {
            lines.push(format!(
                "rustfmt could not format `{output}`; run `cargo fmt`"
            ));
        }
        if let Some(module) = &self.declared {
            lines.push(format!("Declared `pub mod {module};` in `src/lib.rs`"));
        }
        if self.created {
            lines.push(
                "Next, pass your `Admin` through the file's `models` function in `src/app.rs`"
                    .to_string(),
            );
        }
        lines.join("\n")
    }
}

/// The parts of `path` after `src/`, which it must be under, relative and
/// without `..`: `./src/models/_entities` is `["models", "_entities"]`.
fn under_src(path: &Path, what: &str) -> Result<Vec<String>> {
    let invalid = || {
        Error::string(&format!(
            "the {what} `{}` must be a relative path under `src/`",
            path.display()
        ))
    };
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
            _ => return Err(invalid()),
        }
    }
    if parts.len() < 2 || parts[0] != "src" {
        return Err(invalid());
    }
    parts.remove(0);
    Ok(parts)
}

/// The entity modules `mod.rs` in `dir` declares, in its order. Modules that
/// hold no entity (`prelude`, `sea_orm_active_enums`), the admin's own roles
/// table and Loco's own tables are left out.
fn read_entities(dir: &Path, warnings: &mut Vec<String>) -> Result<Vec<Entity>> {
    let index = dir.join("mod.rs");
    let fix = "run `cargo loco db entities` first, or set the entities directory with \
               `AdminModels::entities`";
    let text = fs::read_to_string(&index).map_err(|err| {
        Error::string(&format!("cannot read `{}` ({err}): {fix}", index.display()))
    })?;
    let mut entities = Vec::new();
    for line in text.lines() {
        let Some(module) = line
            .trim()
            .strip_prefix("pub mod ")
            .and_then(|rest| rest.strip_suffix(';'))
            .map(str::trim)
        else {
            continue;
        };
        let file = module.trim_start_matches("r#");
        if file == "prelude" || file == ROLES_TABLE || file.starts_with(LOCO_TABLE_PREFIX) {
            continue;
        }
        let source = fs::read_to_string(dir.join(format!("{file}.rs")))
            .or_else(|_| fs::read_to_string(dir.join(file).join("mod.rs")));
        let Ok(source) = source else {
            warnings.push(format!(
                "skipped `{module}`: found no `{file}.rs` or `{file}/mod.rs` in `{}`",
                dir.display()
            ));
            continue;
        };
        if source.contains("DeriveEntityModel") || source.contains("impl EntityName for Entity") {
            entities.push(Entity {
                module: module.to_string(),
                hidden: hidden(&source),
            });
        }
    }
    if entities.is_empty() {
        return Err(Error::string(&format!(
            "found no entities in `{}`: {fix}",
            index.display()
        )));
    }
    Ok(entities)
}

/// A field of an entity's `Model`.
struct Field<'a> {
    name: &'a str,
    ty: &'a str,
    primary_key: bool,
}

/// The fields of the entity's `Model`, skipping the relation fields of
/// `SeaORM`'s dense format (`HasMany<...>` and the like).
fn model_fields(source: &str) -> Vec<Field<'_>> {
    let Some(start) = source.find("pub struct Model") else {
        return Vec::new();
    };
    let body = &source[start..];
    let Some(open) = body.find('{') else {
        return Vec::new();
    };
    let body = &body[open + 1..];
    let body = &body[..body.find('}').unwrap_or(body.len())];
    let mut fields = Vec::new();
    let mut primary_key = false;
    for line in body.lines().map(str::trim) {
        if line.starts_with("#[") {
            primary_key |= line.contains("primary_key");
            continue;
        }
        let Some((name, ty)) = line.strip_prefix("pub ").and_then(|f| f.split_once(':')) else {
            continue;
        };
        let ty = ty.trim().trim_end_matches(',').trim();
        if !["HasMany<", "HasOne<", "BelongsTo<"]
            .iter()
            .any(|relation| ty.starts_with(relation))
        {
            fields.push(Field {
                name: name.trim().trim_start_matches("r#"),
                ty,
                primary_key,
            });
        }
        primary_key = false;
    }
    fields
}

/// How the admin's hiding of sensitive columns affects the entity.
fn hidden(source: &str) -> Hidden {
    let fields = model_fields(source);
    let sensitive = || fields.iter().filter(|f| is_sensitive(f.name));
    if let Some(field) = sensitive().find(|f| f.primary_key) {
        return Hidden::PrimaryKey(field.name.to_string());
    }
    sensitive()
        .find(|f| !f.ty.starts_with("Option<"))
        .map_or(Hidden::None, |f| Hidden::Required(f.name.to_string()))
}

/// Whether `text` names `module`'s entity as `module::Entity`, on its own:
/// `tags::Entity` is not mentioned by `article_tags::Entity`.
fn mentions(text: &str, module: &str) -> bool {
    let needle = format!("{module}::Entity");
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '#';
    text.match_indices(&needle).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + needle.len()..].chars().next();
        !before.is_some_and(ident) && !after.is_some_and(ident)
    })
}

/// The lines registering `entity`, without indentation.
fn entity_lines(alias: &str, entity: &Entity) -> Vec<String> {
    let path = format!("{alias}::{}::Entity", entity.module);
    match &entity.hidden {
        Hidden::None => vec![format!(".model::<{path}>()")],
        Hidden::Required(field) => vec![
            format!("// `{field}` is hidden and required, so records cannot be created here."),
            format!(".register(muy_loco_admin::ModelAdmin::<{path}>::new().can_create(false))"),
        ],
        Hidden::PrimaryKey(field) => vec![
            format!("// The primary key `{field}` looks sensitive, so this model is left out."),
            format!("// To add it, register it with `.show_fields([\"{field}\"])`."),
            format!("// .model::<{path}>()"),
        ],
    }
}

/// Why lines cannot be added to an existing file.
#[derive(Debug, PartialEq, Eq)]
enum MarkerProblem {
    Missing,
    NotAlone,
}

impl MarkerProblem {
    fn message(&self, file: &str) -> String {
        match self {
            Self::Missing => format!(
                "`{file}` has no `{MODELS_MARKER}` line to add models above; add it on a line of \
                 its own after the models, or set another file with `AdminModels::output`"
            ),
            Self::NotAlone => format!(
                "`{file}` has code before `{MODELS_MARKER}`; put the marker on a line of its own"
            ),
        }
    }
}

/// `text` with lines registering `entities` added above the marker, keeping
/// the file's line endings. rustfmt puts a comment after a method chain at the
/// indentation of the statement, so the lines go one level deeper.
fn insert_above_marker(
    text: &str,
    alias: &str,
    entities: &[&Entity],
) -> std::result::Result<String, MarkerProblem> {
    let at = text.find(MODELS_MARKER).ok_or(MarkerProblem::Missing)?;
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..at];
    if !before.chars().all(char::is_whitespace) {
        return Err(MarkerProblem::NotAlone);
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let indent = format!("{before}    ");
    let mut added = String::new();
    for entity in entities {
        for line in entity_lines(alias, entity) {
            added.push_str(&indent);
            added.push_str(&line);
            added.push_str(newline);
        }
    }
    Ok(format!(
        "{}{added}{}",
        &text[..line_start],
        &text[line_start..]
    ))
}

/// A new models file registering `entities`.
fn new_file(module_path: &str, alias: &str, entities_dir: &Path, entities: &[&Entity]) -> String {
    let body =
        insert_above_marker(&format!("    {MODELS_MARKER}\n"), alias, entities).unwrap_or_default();
    format!(
        "//! The models in the admin, kept up to date by `cargo loco task admin:models`.\n\
         //!\n\
         //! The task adds a line above the marker comment for each entity in\n\
         //! `{dir}` that this file does not mention yet. Comment a line out to\n\
         //! leave that model out of the admin, or change `.model::<...>()` to\n\
         //! `.register(ModelAdmin::<...>::new()...)` to configure it.\n\
         \n\
         use muy_loco_admin::Admin;\n\
         \n\
         use {module_path};\n\
         \n\
         /// Registers the app's models on `admin`.\n\
         #[must_use]\n\
         pub fn models(admin: Admin) -> Admin {{\n    \
             admin\n\
         {body}\
         }}\n",
        dir = entities_dir.display(),
    )
}

/// Format `file` with `rustfmt` for the project's edition, if it is
/// installed. Returns whether it was formatted.
fn rustfmt(root: &Path, file: &str) -> bool {
    let edition = fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|manifest| {
            manifest.lines().find_map(|line| {
                let value = line.trim().strip_prefix("edition")?.trim_start();
                let value = value.strip_prefix('=')?.trim().trim_matches('"');
                (!value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
                    .then(|| value.to_string())
            })
        })
        .unwrap_or_else(|| "2021".to_string());
    Command::new("rustfmt")
        .args(["--edition", &edition, file])
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `line` without a leading `pub` or `pub(...)`.
fn without_visibility(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix("pub(") {
        return rest.find(')').map_or(line, |i| rest[i + 1..].trim_start());
    }
    line.strip_prefix("pub ").map_or(line, str::trim_start)
}

/// The module a top-level `mod name;` line declares, with any visibility.
fn module_declared_by(line: &str) -> Option<&str> {
    let rest = without_visibility(line).strip_prefix("mod ")?;
    Some(rest.trim_end().strip_suffix(';')?.trim())
}

/// Declare `module` in `src/lib.rs` if `src/lib.rs` does not declare it yet,
/// among the other module declarations in the order rustfmt sorts them, above
/// the attributes and doc comments of the one it goes before. Returns the
/// module declared.
fn declare_module(root: &Path, module: &str) -> Result<Option<String>> {
    let lib = root.join("src").join("lib.rs");
    let Ok(text) = fs::read_to_string(&lib) else {
        return Ok(None);
    };
    let declared = text.lines().any(|line| {
        without_visibility(line.trim())
            .strip_prefix("mod ")
            .map(str::trim_start)
            .and_then(|rest| rest.strip_prefix(module))
            .is_some_and(|rest| rest.trim_start().starts_with([';', '{']))
    });
    if declared {
        return Ok(None);
    }
    let mut lines: Vec<&str> = text.lines().collect();
    let modules: Vec<(usize, &str)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| module_declared_by(line).map(|name| (i, name)))
        .collect();
    let at = match modules.iter().find(|(_, name)| *name > module) {
        Some(&(mut at, _)) => {
            while at > 0 {
                let above = lines[at - 1].trim_start();
                if !(above.starts_with("#[")
                    || (above.starts_with("//") && !above.starts_with("//!")))
                {
                    break;
                }
                at -= 1;
            }
            at
        }
        None => modules.last().map_or(lines.len(), |(i, _)| i + 1),
    };
    let declaration = format!("pub mod {module};");
    lines.insert(at, &declaration);
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut text = lines.join(newline);
    text.push_str(newline);
    fs::write(&lib, text).map_err(Error::wrap)?;
    Ok(Some(module.to_string()))
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    /// A project directory in the system temp dir, removed when dropped.
    struct Project(PathBuf);

    impl Project {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "muy-loco-admin-generate-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("src/models/_entities")).unwrap();
            fs::write(dir.join("Cargo.toml"), "[package]\nedition = \"2021\"\n").unwrap();
            Self(dir)
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn read(&self, path: &str) -> String {
            fs::read_to_string(self.0.join(path)).unwrap()
        }

        fn entity(&self, module: &str, fields: &str) {
            self.write(
                &format!("src/models/_entities/{module}.rs"),
                &format!(
                    "use sea_orm::entity::prelude::*;\n\n\
                     #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]\n\
                     #[sea_orm(table_name = \"{module}\")]\n\
                     pub struct Model {{\n    #[sea_orm(primary_key)]\n    pub id: i32,\n{fields}}}\n"
                ),
            );
        }

        fn index(&self, modules: &[&str]) {
            let mut text = "//! `SeaORM` Entity\n\npub mod prelude;\n\n".to_string();
            for module in modules {
                let _ = writeln!(text, "pub mod {module};");
            }
            self.write("src/models/_entities/mod.rs", &text);
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Whether `rustfmt` can run here; CI's test job installs none.
    fn have_rustfmt() -> bool {
        Command::new("rustfmt")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// A SaaS-starter-like app: users with a password, articles, an enum
    /// module, the roles table, Loco's session table and a prelude.
    fn starter(name: &str) -> Project {
        let project = Project::new(name);
        project.entity(
            "users",
            "    pub email: String,\n    pub password: String,\n    pub api_key: Option<String>,\n",
        );
        project.entity("articles", "    pub title: String,\n");
        project.entity("admin_roles", "    pub name: String,\n");
        project.entity("loco_sessions", "    pub data: String,\n");
        project.write(
            "src/models/_entities/sea_orm_active_enums.rs",
            "#[derive(DeriveActiveEnum)]\npub enum Status {}\n",
        );
        project.write(
            "src/models/_entities/prelude.rs",
            "pub use super::users::Entity as Users;\n",
        );
        project.index(&[
            "admin_roles",
            "articles",
            "loco_sessions",
            "sea_orm_active_enums",
            "users",
        ]);
        project.write("src/lib.rs", "pub mod app;\npub mod models;");
        project
    }

    const STARTER_FILE: &str = "//! The models in the admin, kept up to date by `cargo loco task admin:models`.\n\
         //!\n\
         //! The task adds a line above the marker comment for each entity in\n\
         //! `src/models/_entities` that this file does not mention yet. Comment a line out to\n\
         //! leave that model out of the admin, or change `.model::<...>()` to\n\
         //! `.register(ModelAdmin::<...>::new()...)` to configure it.\n\
         \n\
         use muy_loco_admin::Admin;\n\
         \n\
         use crate::models::_entities;\n\
         \n\
         /// Registers the app's models on `admin`.\n\
         #[must_use]\n\
         pub fn models(admin: Admin) -> Admin {\n    \
             admin\n        \
             .model::<_entities::articles::Entity>()\n        \
             // `password` is hidden and required, so records cannot be created here.\n        \
             .register(muy_loco_admin::ModelAdmin::<_entities::users::Entity>::new().can_create(false))\n    \
             // admin-models-inject (do not remove this comment)\n\
         }\n";

    #[test]
    fn the_first_run_creates_the_file_and_declares_its_module() {
        let project = starter("create");
        let report = AdminModels::new().generate(&project.0).unwrap();
        assert!(report.created);
        assert_eq!(report.added, vec!["articles", "users"]);
        assert_eq!(report.declared.as_deref(), Some("admin"));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.formatted, have_rustfmt());
        // Already in rustfmt's layout, so formatting leaves it as written.
        assert_eq!(project.read("src/admin.rs"), STARTER_FILE);
        assert_eq!(
            project.read("src/lib.rs"),
            "pub mod admin;\npub mod app;\npub mod models;\n"
        );
        let summary = Report {
            formatted: true,
            ..report
        }
        .summary(Path::new("src/admin.rs"));
        assert_eq!(
            summary,
            "Created `src/admin.rs` with 2 models: articles, users\n\
             Declared `pub mod admin;` in `src/lib.rs`\n\
             Next, pass your `Admin` through the file's `models` function in `src/app.rs`"
        );
    }

    #[test]
    fn later_runs_add_only_entities_the_file_does_not_mention() {
        let project = starter("update");
        let generator = AdminModels::new();
        generator.generate(&project.0).unwrap();

        let again = generator.generate(&project.0).unwrap();
        assert_eq!(
            again,
            Report::default(),
            "nothing to add, format or declare"
        );
        assert_eq!(
            again.summary(Path::new("src/admin.rs")),
            "`src/admin.rs` already lists every entity"
        );

        // Leave articles out, and add two tables; `article_tags` must not
        // count as a mention of `tags`.
        let file = project.read("src/admin.rs").replace(
            "        .model::<_entities::articles::Entity>()",
            "        // .model::<_entities::articles::Entity>()\n        \
             .model::<_entities::article_tags::Entity>()",
        );
        project.write("src/admin.rs", &file);
        project.entity("article_tags", "    pub tag_id: i32,\n");
        project.entity("tags", "    pub name: String,\n");
        project.index(&["article_tags", "articles", "tags", "users"]);

        let report = generator.generate(&project.0).unwrap();
        assert_eq!(report.added, vec!["tags"]);
        assert!(!report.created);
        let summary = Report {
            formatted: false,
            ..report
        }
        .summary(Path::new("src/admin.rs"));
        assert_eq!(
            summary,
            "Added 1 model to `src/admin.rs`: tags\n\
             rustfmt could not format `src/admin.rs`; run `cargo fmt`"
        );
        let file = project.read("src/admin.rs");
        assert!(
            file.contains(
                "        .model::<_entities::tags::Entity>()\n    \
                 // admin-models-inject (do not remove this comment)\n}"
            ),
            "{file}"
        );
        assert!(file.contains("// .model::<_entities::articles::Entity>()"));
        assert_eq!(file.matches("articles::Entity").count(), 1, "{file}");
    }

    #[test]
    fn rustfmt_formats_what_it_cannot_write_in_its_layout() {
        if !have_rustfmt() {
            return;
        }
        // One short chain collapses onto one line, and a long `register`
        // wraps; formatting keeps `cargo fmt --check` passing.
        let project = Project::new("rustfmt");
        project.entity("service_accounts", "    pub client_secret: String,\n");
        project.index(&["service_accounts"]);
        let report = AdminModels::new().generate(&project.0).unwrap();
        assert!(report.formatted);
        let check = Command::new("rustfmt")
            .args(["--edition", "2021", "--check", "src/admin.rs"])
            .current_dir(&project.0)
            .status()
            .unwrap();
        assert!(check.success(), "{}", project.read("src/admin.rs"));
    }

    #[test]
    fn sensitive_primary_keys_relations_and_raw_identifiers() {
        let project = Project::new("edge-cases");
        project.write(
            "src/models/_entities/sessions.rs",
            "#[derive(DeriveEntityModel)]\npub struct Model {\n    \
             #[sea_orm(primary_key, auto_increment = false)]\n    pub token: String,\n    \
             pub user_id: i32,\n}\n",
        );
        project.entity(
            "accounts",
            "    #[sea_orm(has_many)]\n    pub api_tokens: HasMany<super::api_tokens::Entity>,\n",
        );
        project.entity("type", "    pub name: String,\n");
        project.index(&["accounts", "missing", "sessions", "r#type"]);
        let report = AdminModels::new().generate(&project.0).unwrap();
        assert_eq!(report.added, vec!["accounts", "sessions", "r#type"]);
        assert_eq!(
            report.warnings,
            vec![format!(
                "skipped `missing`: found no `missing.rs` or `missing/mod.rs` in `{}`",
                project.0.join("src/models/_entities").display()
            )]
        );
        let file = project.read("src/admin.rs");
        assert!(
            file.contains(".model::<_entities::accounts::Entity>()"),
            "{file}"
        );
        assert!(
            file.contains(".model::<_entities::r#type::Entity>()"),
            "{file}"
        );
        assert!(
            file.contains(
                "        // The primary key `token` looks sensitive, so this model is left out.\n        \
                 // To add it, register it with `.show_fields([\"token\"])`.\n        \
                 // .model::<_entities::sessions::Entity>()\n"
            ),
            "{file}"
        );
        assert!(
            report
                .summary(Path::new("src/admin.rs"))
                .starts_with("warning: skipped")
        );

        // The commented-out line counts as a mention, so it stays out.
        let again = AdminModels::new().generate(&project.0).unwrap();
        assert_eq!(again.added, Vec::<String>::new());
    }

    #[test]
    fn module_declarations_keep_attributes_and_visibility() {
        let project = starter("modules");
        project.write(
            "src/lib.rs",
            "//! The app.\r\n#[cfg(feature = \"x\")]\r\npub mod app;\r\n/// Models.\r\npub mod models;\r\n",
        );
        AdminModels::new().generate(&project.0).unwrap();
        assert_eq!(
            project.read("src/lib.rs"),
            "//! The app.\r\npub mod admin;\r\n#[cfg(feature = \"x\")]\r\npub mod app;\r\n\
             /// Models.\r\npub mod models;\r\n"
        );

        let project = starter("visibility");
        project.write("src/lib.rs", "pub(crate) mod admin;\n");
        let report = AdminModels::new().generate(&project.0).unwrap();
        assert_eq!(report.declared, None);
        assert_eq!(project.read("src/lib.rs"), "pub(crate) mod admin;\n");
    }

    #[test]
    fn custom_paths_and_existing_modules() {
        let project = starter("paths");
        project.write("src/lib.rs", "mod backoffice {}\n");
        let generator = AdminModels::new()
            .entities("./src/models/_entities")
            .output("./src/backoffice.rs");
        let report = generator.generate(&project.0).unwrap();
        assert!(report.created);
        assert_eq!(report.declared, None, "already declared inline");
        assert!(project.read("src/backoffice.rs").contains("pub fn models"));

        // Outside `src/`, the module is the app's to declare.
        let nested = AdminModels::new().output("src/admin/models.rs");
        assert_eq!(nested.generate(&project.0).unwrap().declared, None);
        assert_eq!(project.read("src/lib.rs"), "mod backoffice {}\n");

        // After the last module, or at the end without any.
        project.write(
            "src/lib.rs",
            "//! The app.\n\npub mod app;\nmod models;\n\nfn main() {}",
        );
        let last = AdminModels::new().output("src/web_admin.rs");
        let declared = last.generate(&project.0).unwrap().declared;
        assert_eq!(declared.as_deref(), Some("web_admin"));
        assert_eq!(
            project.read("src/lib.rs"),
            "//! The app.\n\npub mod app;\nmod models;\npub mod web_admin;\n\nfn main() {}\n"
        );
        project.write("src/lib.rs", "//! The app.");
        let none = AdminModels::new().output("./src/staff.rs");
        none.generate(&project.0).unwrap();
        assert_eq!(project.read("src/lib.rs"), "//! The app.\npub mod staff;\n");
    }

    #[test]
    fn errors_name_the_fix() {
        let project = starter("errors");
        let generate =
            |generator: AdminModels| generator.generate(&project.0).unwrap_err().to_string();

        project.write("src/admin.rs", "pub fn admin() {}\n");
        let err = generate(AdminModels::new());
        assert!(err.contains("has no `// admin-models-inject"), "{err}");

        project.write(
            "src/admin.rs",
            "    admin.model::<x::Entity>() // admin-models-inject (do not remove this comment)\n",
        );
        let err = generate(AdminModels::new());
        assert!(err.contains("put the marker on a line of its own"), "{err}");
        assert!(
            project.read("src/admin.rs").starts_with("    admin.model"),
            "unchanged"
        );

        let err = generate(AdminModels::new().entities("src/missing"));
        assert!(err.contains("run `cargo loco db entities` first"), "{err}");

        project.index(&["sea_orm_active_enums"]);
        let err = generate(AdminModels::new());
        assert!(err.contains("found no entities in"), "{err}");

        for dir in ["models/_entities", "src", "/src/models", "src/../models"] {
            let err = generate(AdminModels::new().entities(dir));
            assert!(
                err.contains("must be a relative path under `src/`"),
                "{dir}: {err}"
            );
        }
        let err = generate(AdminModels::new().output("../admin.rs"));
        assert!(
            err.contains("the output file `../admin.rs` must be"),
            "{err}"
        );
    }

    #[test]
    fn mentions_match_whole_names_only() {
        assert!(mentions(".model::<users::Entity>()", "users"));
        assert!(mentions("ModelAdmin::<crate::users::Entity>", "users"));
        assert!(!mentions(".model::<article_tags::Entity>()", "tags"));
        assert!(!mentions(".model::<tags::EntityExt>()", "tags"));
        assert!(!mentions(".model::<r#type::Entity>()", "type"));
        assert!(mentions(".model::<r#type::Entity>()", "r#type"));
        assert!(!mentions("", "tags"));
    }

    #[test]
    fn hidden_columns_are_found_in_the_model() {
        let source = "pub struct Model {\n    pub id: i32,\n    pub r#token: String,\n}\n\
                      pub struct Other {\n    pub password: String,\n}\n";
        assert_eq!(hidden(source), Hidden::Required("token".to_string()));
        assert_eq!(
            hidden("pub struct Model {\n    pub secret: Option<String>,\n}"),
            Hidden::None
        );
        assert_eq!(hidden("pub enum Relation {}"), Hidden::None);
        assert_eq!(hidden("pub struct Model"), Hidden::None);
    }
}
