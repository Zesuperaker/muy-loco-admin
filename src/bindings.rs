//! Generates `frontend/src/bindings.ts`: the TypeScript types of the API's
//! request and response bodies, from the Rust types that define them.
//!
//! The types derive `ts_rs::TS` in test builds only. The test below writes the
//! file whenever it is out of date and then fails, so a change to an API type
//! fails CI until the regenerated file is committed.

use std::{any::TypeId, collections::BTreeMap, collections::HashSet, fs, path::Path};

use ts_rs::{Config, TS, TypeVisitor};

use crate::{api, error::ErrorBody, roles::RoleInput};

const OUTPUT: &str = "frontend/src/bindings.ts";

/// The declarations of the visited types and of every type they refer to
/// by name, keyed by TypeScript name.
struct Declarations<'a> {
    cfg: &'a Config,
    seen: HashSet<TypeId>,
    decls: BTreeMap<String, String>,
}

impl TypeVisitor for Declarations<'_> {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        // Types without an output path (strings, arrays, maps...) are inlined
        // where they are used; their generic arguments are visited separately.
        if T::output_path().is_none() || !self.seen.insert(TypeId::of::<T>()) {
            return;
        }
        let name = T::ident(self.cfg);
        let decl = format!(
            "{}export {}",
            T::docs().unwrap_or_default(),
            T::decl(self.cfg)
        );
        // ts-rs leaves a space at the end of a line before each field's docs.
        let decl = decl
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        // `serde_json::Value` and the type ts-rs declares it with are distinct
        // Rust types with the same declaration.
        if let Some(other) = self.decls.insert(name.clone(), decl.clone()) {
            assert_eq!(other, decl, "two types are named `{name}` in TypeScript");
        }
        T::visit_dependencies(self);
    }
}

fn generate() -> String {
    // The API's integers fit in a JavaScript number.
    let cfg = Config::new().with_large_int("number");
    let mut types = Declarations {
        cfg: &cfg,
        seen: HashSet::new(),
        decls: BTreeMap::new(),
    };
    types.visit::<api::AdminConfig>();
    types.visit::<api::CurrentUser>();
    types.visit::<api::SchemaResponse<'static>>();
    types.visit::<api::StatsResponse>();
    types.visit::<api::ListResponse>();
    types.visit::<api::CountResponse>();
    types.visit::<api::RecordResponse>();
    types.visit::<api::DeleteResponse>();
    types.visit::<api::ActionResponse>();
    types.visit::<api::LookupResponse>();
    types.visit::<api::RolesResponse>();
    types.visit::<api::RoleResponse>();
    types.visit::<api::KeysBody>();
    types.visit::<RoleInput>();
    types.visit::<ErrorBody>();

    let mut out = String::from(
        "// Generated from the crate's Rust types by `src/bindings.rs`; do not edit.\n\
         // `cargo test` regenerates it.\n",
    );
    for decl in types.decls.values() {
        out.push('\n');
        out.push_str(decl);
        out.push('\n');
    }
    out
}

#[test]
fn typescript_bindings_are_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(OUTPUT);
    // The published crate does not include the frontend's sources.
    if !path.parent().is_some_and(Path::exists) {
        return;
    }
    let generated = generate();
    if fs::read_to_string(&path).ok().as_deref() != Some(generated.as_str()) {
        fs::write(&path, generated).expect("write the TypeScript bindings");
        panic!("{OUTPUT} was out of date and has been regenerated; commit it");
    }
}
