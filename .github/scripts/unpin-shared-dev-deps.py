#!/usr/bin/env python3
"""Let the tests run with other versions of the crate's dependencies.

The dev-dependencies pin exact versions, and Cargo resolves a crate that is
both a dependency and a dev-dependency (loco-rs, sea-orm, tokio) to one
version, so the pins keep those dependencies at one version in every test
run. This gives each of them the crate's own requirement in
`[dev-dependencies]`, so `cargo update` can resolve them to their floor or
their latest release. CI runs it before those jobs; it rewrites `Cargo.toml`,
so do not commit the result.
"""

import re
import tomllib

MANIFEST = "Cargo.toml"

with open(MANIFEST, "rb") as file:
    manifest = tomllib.load(file)
dependencies = manifest["dependencies"]
with open(MANIFEST, encoding="utf-8") as file:
    head, table, dev = file.read().partition("[dev-dependencies]")

for name in sorted(set(dependencies) & set(manifest["dev-dependencies"])):
    dependency = dependencies[name]
    version = dependency if isinstance(dependency, str) else dependency["version"]
    # `name = "=1.2.3"` or `name = { version = "=1.2.3", ...`
    entry = rf'(?m)^({re.escape(name)} = (?:\{{ version = )?)"[^"]*"'
    dev, count = re.subn(entry, rf'\g<1>"{version}"', dev)
    if count != 1:
        raise SystemExit(f"cannot find the version of `{name}` in [dev-dependencies]")
    print(f"{name}: tests use the crate's requirement, {version}")

with open(MANIFEST, "w", encoding="utf-8") as file:
    file.write(head + table + dev)
