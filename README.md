# leafdb

A lightweight, **browser-first relational database** written in Rust and
compiled to WebAssembly. leafdb runs entirely inside the browser, stores its
data in the [Origin Private File System (OPFS)](https://developer.mozilla.org/docs/Web/API/File_System_API/Origin_private_file_system),
and exposes a small SQL surface to JavaScript/TypeScript.

leafdb is a learning-oriented but genuinely usable engine. Its goals and
non-goals are recorded in the ADRs:

- [ADR-0001 — ブラウザ向け軽量RDBをRustとWebAssemblyで実装する](docs/adr/0001-browser-first-rdb-in-rust-and-wasm.md)
- [ADR-0002 — ページベースのストレージ管理を採用する](docs/adr/0002-page-based-storage.md)

## Quick start (JavaScript / TypeScript)

```ts
import { Database } from "./web/leafdb.js";

const db = await Database.open("app");

await db.exec(`
  CREATE TABLE todos (
    id INTEGER PRIMARY KEY,
    title TEXT,
    done BOOLEAN
  )
`);

await db.exec("INSERT INTO todos VALUES (?, ?, ?)", [1, "Build a database", false]);

const rows = await db.query("SELECT * FROM todos WHERE done = ?", [false]);
// -> [{ id: 1, title: "Build a database", done: false }]
```

Reloading the page restores the data from OPFS — no server required.

## Architecture

```text
JavaScript / TypeScript  (web/leafdb.js — async API + OPFS persistence)
          │
          ▼
       WASM API           (crates/leafdb-wasm — wasm-bindgen bindings)
          │
          ▼
   Rust RDB Engine        (crates/leafdb-core)
   ├─ SQL parser / executor
   ├─ Catalog (page 0)
   ├─ Table heap (slotted pages, chained)
   └─ Page manager
          │
          ▼
       Storage            (Storage trait; MemoryStorage today)
```

Per ADR-0002, the engine only ever talks to a page-based `Storage` trait and
never touches OPFS directly. Persistence works by exporting the raw page image
and writing it to OPFS from JavaScript; on open, the image is read back and
handed to the engine.

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/leafdb-core` | Storage, pages, catalog, SQL parser/executor, `Database` |
| `crates/leafdb-wasm` | `wasm-bindgen` bindings (`LeafDb`: `exec`/`query`/`export`) |
| `web/` | JS/TS API wrapper (`leafdb.js`), types, and a demo page |
| `docs/adr/` | Architecture Decision Records |

## Building & testing

Prerequisites: a Rust toolchain, the `wasm32-unknown-unknown` target, and
[`wasm-pack`](https://rustwasm.github.io/wasm-pack/).

```bash
# Native engine tests (fast, no browser needed)
cargo test

# Build the WebAssembly package into web/pkg
wasm-pack build crates/leafdb-wasm --target web --out-dir ../../web/pkg --out-name leafdb
# or:
./scripts/build-wasm.sh

# Serve the demo (OPFS needs a secure context; localhost qualifies)
python3 -m http.server 8080 --directory web
# open http://localhost:8080/
```

## Supported SQL (initial scope)

- `CREATE TABLE name (col TYPE [PRIMARY KEY], ...)` with `IF NOT EXISTS`
- `INSERT INTO name [(cols)] VALUES (...), (...)`
- `SELECT * | cols FROM name [WHERE expr]`
- `UPDATE name SET col = val, ... [WHERE expr]`
- `DELETE FROM name [WHERE expr]`
- Types: `INTEGER`, `TEXT`, `BOOLEAN` (plus `NULL`)
- `WHERE` expressions: `= != <> < <= > >=`, `AND`, `OR`, parentheses, and `?`
  positional parameters

## Status & roadmap

Implemented today: page-based storage, a slotted-page table heap with page
chaining, a single-page catalog, the SQL subset above, primary-key uniqueness
(enforced by heap scan), and OPFS persistence via image export/import.

Deferred to later ADRs (explicitly out of the initial scope): B+Tree indexes,
a page cache, WAL and crash recovery, MVCC transactions, and query
optimization. The page-based design is chosen precisely so these can be layered
on later.

## License

Licensed under either of MIT or Apache-2.0 at your option.
