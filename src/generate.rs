//! The `admin:models` task, which keeps a file listing the app's models in the admin.

use std::{
    fs,
    path::{Component, Path, PathBuf},
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

/// `cargo loco task admin:models` adds every `SeaORM` entity of the app to a
/// file listing the models in the admin, `src/admin.rs` by default.
///
/// The first run creates the file, with a `models` function that registers
/// each entity in `src/models/_entities`, and declares its module in
/// `src/lib.rs`. Later runs add only the entities the file does not mention
/// yet, above [`MODELS_MARKER`], so a line you comment out stays out of the
/// admin and a model you configure with [`ModelAdmin`](crate::ModelAdmin)
/// keeps its configuration. An entity whose required column is hidden, like
/// `users.password`, is added with `can_create(false)`, since records could
/// not be created from the admin.
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
    module: String,
    /// A hidden column that has no default value in Rust (not an `Option`),
    /// which the admin's create form could not fill in.
    required_hidden: Option<String>,
}

/// What a run changed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Report {
    created: bool,
    added: Vec<String>,
    declared: Option<String>,
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

    /// The file listing the models, relative to the project root (defaults to
    /// `src/admin.rs`). Its module is declared in `src/lib.rs` only when it is
    /// directly in `src/`.
    #[must_use]
    pub fn output(mut self, file: impl Into<PathBuf>) -> Self {
        self.output = file.into();
        self
    }

    /// Create or update the output file under the project root `root`.
    fn generate(&self, root: &Path) -> Result<Report> {
        let module_path = rust_module_path(&self.entities)?;
        let alias = module_path
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .to_string();
        let entities = read_entities(&root.join(&self.entities))?;
        let output = root.join(&self.output);
        let mut report = Report::default();

        if output.exists() {
            let text = fs::read_to_string(&output).map_err(Error::wrap)?;
            let missing: Vec<&Entity> = entities
                .iter()
                .filter(|entity| !mentions(&text, &entity.module))
                .collect();
            if !missing.is_empty() {
                let updated = insert_above_marker(&text, &alias, &missing).ok_or_else(|| {
                    Error::string(&format!(
                        "`{}` has no `{MODELS_MARKER}` line to add models above; add it at the \
                         end of the models, or set another file with `AdminModels::output`",
                        self.output.display()
                    ))
                })?;
                fs::write(&output, updated).map_err(Error::wrap)?;
                report.added = missing.iter().map(|e| e.module.clone()).collect();
            }
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent).map_err(Error::wrap)?;
            }
            let all: Vec<&Entity> = entities.iter().collect();
            fs::write(
                &output,
                new_file(&module_path, &alias, &self.entities, &all),
            )
            .map_err(Error::wrap)?;
            report.created = true;
            report.added = entities.iter().map(|e| e.module.clone()).collect();
        }

        report.declared = declare_module(root, &self.output)?;
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
        let mut lines = Vec::new();
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

/// The Rust path of the module in `dir`, which must be under `src/`:
/// `src/models/_entities` is `crate::models::_entities`.
fn rust_module_path(dir: &Path) -> Result<String> {
    let mut parts = dir.components().filter(|c| !matches!(c, Component::CurDir));
    let invalid = || {
        Error::string(&format!(
            "the entities directory `{}` must be a relative path under `src/`",
            dir.display()
        ))
    };
    if parts.next() != Some(Component::Normal("src".as_ref())) {
        return Err(invalid());
    }
    let mut path = "crate".to_string();
    for part in parts {
        let Component::Normal(name) = part else {
            return Err(invalid());
        };
        path.push_str("::");
        path.push_str(&name.to_string_lossy());
    }
    if path == "crate" {
        return Err(invalid());
    }
    Ok(path)
}

/// The entity modules `mod.rs` in `dir` declares, in its order. Modules that
/// hold no entity (`prelude`, `sea_orm_active_enums`) and the admin's own
/// roles table, which cannot be registered, are left out.
fn read_entities(dir: &Path) -> Result<Vec<Entity>> {
    let index = dir.join("mod.rs");
    let text = fs::read_to_string(&index).map_err(|err| {
        Error::string(&format!(
            "cannot read `{}` ({err}): run `cargo loco db entities` first, or set the entities \
             directory with `AdminModels::entities`",
            index.display()
        ))
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
        if module == ROLES_TABLE {
            continue;
        }
        let source = fs::read_to_string(dir.join(format!("{module}.rs")))
            .or_else(|_| fs::read_to_string(dir.join(module).join("mod.rs")));
        let Ok(source) = source else { continue };
        if source.contains("DeriveEntityModel") || source.contains("impl EntityName for Entity") {
            entities.push(Entity {
                module: module.to_string(),
                required_hidden: required_hidden(&source),
            });
        }
    }
    Ok(entities)
}

/// The first field of the entity's `Model` that the admin hides and that is
/// not an `Option`, so a new record could not leave it empty.
fn required_hidden(source: &str) -> Option<String> {
    let start = source.find("pub struct Model")?;
    let body = &source[start..];
    let body = &body[body.find('{')? + 1..];
    let body = &body[..body.find('}')?];
    body.lines().find_map(|line| {
        let (name, ty) = line.trim().strip_prefix("pub ")?.split_once(':')?;
        let name = name.trim().trim_start_matches("r#");
        (is_sensitive(name) && !ty.trim().starts_with("Option<")).then(|| name.to_string())
    })
}

/// Whether `text` names `module`'s entity as `module::Entity`, on its own:
/// `tags::Entity` is not mentioned by `article_tags::Entity`.
fn mentions(text: &str, module: &str) -> bool {
    let needle = format!("{module}::Entity");
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(&needle).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + needle.len()..].chars().next();
        !before.is_some_and(ident) && !after.is_some_and(ident)
    })
}

/// The lines registering `entity`, without indentation.
fn entity_lines(alias: &str, entity: &Entity) -> Vec<String> {
    let path = format!("{alias}::{}::Entity", entity.module);
    match &entity.required_hidden {
        None => vec![format!(".model::<{path}>()")],
        Some(field) => vec![
            format!("// `{field}` is hidden and required, so records cannot be created here."),
            format!(".register(muy_loco_admin::ModelAdmin::<{path}>::new().can_create(false))"),
        ],
    }
}

/// `text` with lines registering `entities` added above the marker, or `None`
/// without a marker. rustfmt puts a comment after a method chain at the
/// indentation of the statement, so the lines go one level deeper.
fn insert_above_marker(text: &str, alias: &str, entities: &[&Entity]) -> Option<String> {
    let at = text.find(MODELS_MARKER)?;
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let indent = format!("{}    ", &text[line_start..at]);
    let mut added = String::new();
    for entity in entities {
        for line in entity_lines(alias, entity) {
            added.push_str(&indent);
            added.push_str(&line);
            added.push('\n');
        }
    }
    Some(format!(
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

/// Declare `output`'s module in `src/lib.rs` when it is a file directly in
/// `src/` that `src/lib.rs` does not declare yet, among the other module
/// declarations in the order rustfmt sorts them. Returns the module declared.
fn declare_module(root: &Path, output: &Path) -> Result<Option<String>> {
    if output.parent() != Some(Path::new("src")) {
        return Ok(None);
    }
    let Some(module) = output.file_stem().and_then(|s| s.to_str()) else {
        return Ok(None);
    };
    let lib = root.join("src").join("lib.rs");
    let Ok(text) = fs::read_to_string(&lib) else {
        return Ok(None);
    };
    let declared = text.lines().any(|line| {
        let line = line.trim();
        let line = line.strip_prefix("pub ").unwrap_or(line);
        line.strip_prefix("mod ")
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
    let at = modules
        .iter()
        .find(|(_, name)| *name > module)
        .map(|(i, _)| *i)
        .or_else(|| modules.last().map(|(i, _)| i + 1))
        .unwrap_or(lines.len());
    let declaration = format!("pub mod {module};");
    lines.insert(at, &declaration);
    let mut text = lines.join("\n");
    text.push('\n');
    fs::write(&lib, text).map_err(Error::wrap)?;
    Ok(Some(module.to_string()))
}

/// The module a top-level `mod name;` or `pub mod name;` line declares.
fn module_declared_by(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("pub ")
        .unwrap_or(line)
        .strip_prefix("mod ")?;
    Some(rest.trim_end().strip_suffix(';')?.trim())
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

    /// A SaaS-starter-like app: users with a password, articles, an enum
    /// module, the roles table and a prelude.
    fn starter(name: &str) -> Project {
        let project = Project::new(name);
        project.entity(
            "users",
            "    pub email: String,\n    pub password: String,\n    pub api_key: Option<String>,\n",
        );
        project.entity("articles", "    pub title: String,\n");
        project.entity("admin_roles", "    pub name: String,\n");
        project.write(
            "src/models/_entities/sea_orm_active_enums.rs",
            "#[derive(DeriveActiveEnum)]\npub enum Status {}\n",
        );
        project.write(
            "src/models/_entities/prelude.rs",
            "pub use super::users::Entity as Users;\n",
        );
        project.index(&["admin_roles", "articles", "sea_orm_active_enums", "users"]);
        project.write("src/lib.rs", "pub mod app;\npub mod models;");
        project
    }

    #[test]
    fn the_first_run_creates_the_file_and_declares_its_module() {
        let project = starter("create");
        let report = AdminModels::new().generate(&project.0).unwrap();
        assert_eq!(
            report,
            Report {
                created: true,
                added: vec!["articles".into(), "users".into()],
                declared: Some("admin".into()),
            }
        );
        assert_eq!(
            project.read("src/admin.rs"),
            "//! The models in the admin, kept up to date by `cargo loco task admin:models`.\n\
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
             }\n"
        );
        assert_eq!(
            project.read("src/lib.rs"),
            "pub mod admin;\npub mod app;\npub mod models;\n"
        );
        assert_eq!(
            report.summary(Path::new("src/admin.rs")),
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
        assert_eq!(again, Report::default(), "nothing to add or declare");
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
        assert_eq!(report.added, vec!["tags".to_string()]);
        assert!(!report.created);
        assert_eq!(
            report.summary(Path::new("src/admin.rs")),
            "Added 1 model to `src/admin.rs`: tags"
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
    fn custom_paths_and_existing_modules() {
        let project = starter("paths");
        project.write("src/lib.rs", "mod backoffice {}\n");
        let generator = AdminModels::new()
            .entities("./src/models/_entities")
            .output("src/backoffice.rs");
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
        assert_eq!(
            last.generate(&project.0).unwrap().declared.as_deref(),
            Some("web_admin")
        );
        assert_eq!(
            project.read("src/lib.rs"),
            "//! The app.\n\npub mod app;\nmod models;\npub mod web_admin;\n\nfn main() {}\n"
        );
        project.write("src/lib.rs", "//! The app.");
        let none = AdminModels::new().output("src/staff.rs");
        none.generate(&project.0).unwrap();
        assert_eq!(project.read("src/lib.rs"), "//! The app.\npub mod staff;\n");
    }

    #[test]
    fn errors_name_the_fix() {
        let project = starter("errors");
        project.write("src/admin.rs", "pub fn admin() {}\n");
        let err = AdminModels::new()
            .generate(&project.0)
            .unwrap_err()
            .to_string();
        assert!(err.contains("has no `// admin-models-inject"), "{err}");

        let err = AdminModels::new()
            .entities("src/missing")
            .generate(&project.0)
            .unwrap_err()
            .to_string();
        assert!(err.contains("run `cargo loco db entities` first"), "{err}");

        for dir in ["models/_entities", "src", "/src/models", "src/../models"] {
            let err = AdminModels::new()
                .entities(dir)
                .generate(&project.0)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("must be a relative path under `src/`"),
                "{dir}: {err}"
            );
        }
    }

    #[test]
    fn mentions_match_whole_names_only() {
        assert!(mentions(".model::<users::Entity>()", "users"));
        assert!(mentions("ModelAdmin::<crate::users::Entity>", "users"));
        assert!(!mentions(".model::<article_tags::Entity>()", "tags"));
        assert!(!mentions(".model::<tags::EntityExt>()", "tags"));
        assert!(!mentions("", "tags"));
    }

    #[test]
    fn required_hidden_fields_are_found_in_the_model() {
        let source = "pub struct Model {\n    pub id: i32,\n    pub r#token: String,\n}\n\
                      pub struct Other {\n    pub password: String,\n}\n";
        assert_eq!(required_hidden(source), Some("token".to_string()));
        assert_eq!(
            required_hidden("pub struct Model {\n    pub secret: Option<String>,\n}"),
            None
        );
        assert_eq!(required_hidden("pub enum Relation {}"), None);
    }
}
