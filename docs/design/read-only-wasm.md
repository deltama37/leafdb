# 設計書: 読み取り専用 WASM と一括ロード

## 目的

- WASM から公開する API を読み取り専用にする。データは JavaScript からテーブル定義と行として一度だけ受け取り（一括ロード）、以降は `SELECT` だけを受け付ける。
- OPFS を「書き込みの保存先」から「組み立て済みイメージのキャッシュ」に変える。キャッシュのキーはアプリが渡すデータのバージョンとする。
- キャッシュから読むイメージは壊れている可能性があるため、壊れたイメージや巨大な長さ値で panic / abort しないようにする（WASM では panic がインスタンスを復旧不能にする）。

## 対象外

- 事前ビルド（ネイティブでイメージを作って配る CLI 等）
- Service Worker / PWA マニフェスト、HTTP の条件付きリクエスト（ETag の扱いはアプリ側の責務。アプリは `version` を渡すだけ）
- SQL の拡張（`ORDER BY`、`LIMIT`、集約など）、インデックス（B+Tree）
- Web Worker 化、結果の受け渡しの高速化（`result_to_js` の方式は現状維持）
- ブラウザ上での自動テスト（`wasm-bindgen-test`）。WASM 層は親エージェントがブラウザで手動確認する
- `AGENTS.md`、`Makefile`、`.cursor/`、`scripts/` の変更

## 関連する ADR

- [ADR-001](../adr/0001-browser-first-rdb-in-rust-and-wasm.md)（一部を ADR-003 で置き換え）
- [ADR-002](../adr/0002-page-based-storage.md)（ページベースのストレージは維持）
- [ADR-003](../adr/0003-read-only-wasm-with-bulk-load.md)（本設計の方針）

## 実装単位

実装は 2 単位に分け、単位 1 → 単位 2 の順に実施する。

| 単位 | 範囲 | 完了条件 |
| --- | --- | --- |
| 1 | `crates/leafdb-core` のすべて（テスト含む） | `cargo fmt --all -- --check`、`cargo clippy -p leafdb-core --all-targets -- -D warnings`、`cargo test -p leafdb-core` が通る |
| 2 | `crates/leafdb-wasm`、`web/`、`README.md`、ADR-001 の Status 行 | `make check` が通る |

単位 1 の時点では `leafdb-wasm` が旧 API を使っているためワークスペース全体のビルドは失敗してよい。

---

## 単位 1: `crates/leafdb-core`

### 変更・追加するファイル

| ファイル | 変更 |
| --- | --- |
| `src/error.rs` | `ReadOnly` バリアント追加、`Error::context` 追加 |
| `src/bytes.rs` | `ByteReader::take` の境界確認をオーバーフロー安全にする |
| `src/record.rs` | `decode_row` の事前確保量に上限を付ける |
| `src/catalog.rs` | フォーマットバージョンを追加、事前確保量に上限、`get_mut` を削除 |
| `src/page.rs` | `update` / `delete` を削除、`cell` を境界確認付きの `Result` に変更 |
| `src/heap.rs` | `update_row` / `delete_row` / `RowLocation` / `LocatedRow` を削除、`scan` の戻り値変更と循環検出、`expect` を除去 |
| `src/sql/ast.rs` | `SELECT` 以外の文の型を削除 |
| `src/sql/parser.rs` | `SELECT` だけを解析し、書き込み系キーワードは `ReadOnly` で拒否 |
| `src/executor.rs` | `SELECT` の実行だけを残す。`ExecResult` を `QueryResult` に置き換え |
| `src/load.rs` | 新規。一括ロード |
| `src/database.rs` | `load` / `open_image` / `query` / `export_image` / `tables` だけにする |
| `src/lib.rs` | モジュールと再エクスポート、クレートのドキュメントと doc テストを更新 |
| `tests/engine.rs` | 全面的に書き直す（テスト観点を参照） |

`src/storage.rs`、`src/value.rs`、`src/sql/token.rs`、`src/sql/mod.rs` は変更しない。

### 公開する型・関数のシグネチャ

```rust
// src/error.rs
pub enum Error {
    Parse(String),
    Catalog(String),
    Type(String),
    Constraint(String),
    Parameter(String),
    Storage(String),
    /// A statement other than SELECT was given; leafdb is read-only.
    ReadOnly(String),
}

impl Error {
    /// Returns the same variant with `"{ctx}: "` prepended to the message.
    pub fn context(self, ctx: impl core::fmt::Display) -> Error;
}
// Display: ReadOnly(m) => "read-only: {m}"（他のバリアントの表示は現状維持）

// src/catalog.rs
/// Version of the on-disk image layout. Bump when the format changes.
pub const FORMAT_VERSION: u32 = 1;

// src/page.rs
impl<'a> SlottedPage<'a> {
    pub fn cell(&self, idx: usize) -> Result<Option<&[u8]>>;
    // new / format / next / set_next / num_slots / free_space / insert は現状維持
}

// src/heap.rs
pub fn insert_row(storage: &mut dyn Storage, table: &mut TableSchema, values: &[Value]) -> Result<()>;
pub fn scan(storage: &dyn Storage, table: &TableSchema) -> Result<Vec<Vec<Value>>>;

// src/sql/ast.rs（残す型）
pub enum Projection { All, Columns(Vec<String>) }
pub struct Select { pub projection: Projection, pub table: String, pub filter: Option<Expr> }
pub enum Expr { Literal(Value), Param, Column(String), Binary { left: Box<Expr>, op: BinOp, right: Box<Expr> } }
pub enum BinOp { Eq, NotEq, Lt, LtEq, Gt, GtEq, And, Or }

// src/sql/parser.rs
pub fn parse(sql: &str) -> Result<Select>;

// src/executor.rs
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}
pub fn query(catalog: &Catalog, storage: &dyn Storage, select: Select, params: &[Value]) -> Result<QueryResult>;

// src/load.rs
/// One table's definition and rows for [`crate::Database::load`].
#[derive(Debug, Clone, PartialEq)]
pub struct TableData {
    pub name: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
}
/// True if `s` can be used as a table or column name in SQL.
pub fn is_valid_identifier(s: &str) -> bool;
pub(crate) fn build(tables: Vec<TableData>) -> Result<(MemoryStorage, Catalog)>;

// src/database.rs
pub struct Database { storage: MemoryStorage, catalog: Catalog }
impl Database {
    pub fn load(tables: Vec<TableData>) -> Result<Database>;
    pub fn open_image(image: Vec<u8>) -> Result<Database>;
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<QueryResult>;
    pub fn export_image(&self) -> Vec<u8>;
    pub fn tables(&self) -> &[TableSchema];
}

// src/lib.rs（再エクスポート）
pub use catalog::{Column, TableSchema};
pub use database::Database;
pub use error::{Error, Result};
pub use executor::QueryResult;
pub use load::TableData;
pub use storage::{MemoryStorage, Storage, PAGE_SIZE};
pub use value::{DataType, Value};
```

削除するもの: `Database::create`、`open_or_create`、`execute`、`execute_str`、`flush_catalog`、`ExecResult`、`executor::describe_columns`、`eval_const`、`pk_exists`、`exec_create_table` / `exec_insert` / `exec_update` / `exec_delete`、`Statement`、`CreateTable`、`ColumnDef`、`Insert`、`Update`、`Delete`、`Catalog::get_mut`、`SlottedPage::update` / `delete`、`heap::update_row` / `delete_row`、`RowLocation`、`LocatedRow`。

### 処理の流れ

#### `Error::context`

```text
match self {
  Parse(m)      => Parse(format!("{ctx}: {m}")),
  ...（全バリアントで同じ形）
}
```

#### `ByteReader::take`（bytes.rs）

```text
end = self.pos.checked_add(n)
if end が None または end > buf.len() → Err(Storage("unexpected end of buffer while decoding"))
out = buf[pos..end]; pos = end
```

wasm32 では `usize` が 32 ビットで、release ビルドでは加算が折り返すため `checked_add` が必須。

#### `decode_row`（record.rs）

`Vec::with_capacity(n)` を `Vec::with_capacity(n.min(cell.len()))` に変える。それ以外は現状維持。

#### カタログ（catalog.rs）

ページ 0 の先頭を `MAGIC (u32)`、`FORMAT_VERSION (u32)`、テーブル数 (u32)、… の順にする（`FORMAT_VERSION` を MAGIC の直後に挿入。以降は現状と同じ）。

```text
from_page:
  magic != MAGIC → Err(Storage("database image has an invalid magic header"))   ※現状維持
  version = r.u32()?
  version != FORMAT_VERSION → Err(Storage(format!("unsupported image format version {version} (expected {FORMAT_VERSION})")))
  テーブル用・列用の Vec::with_capacity(x) は x.min(PAGE_SIZE) にする
```

#### `SlottedPage::cell`（page.rs）

```text
if idx >= num_slots()             → Ok(None)
if slot_dir_end() > PAGE_SIZE     → Err(Storage("corrupt page: slot directory overflows the page"))
(off, len) = slot(idx)
if off == 0                       → Ok(None)          // 旧形式の削除済みスロット。読み飛ばす
if off < slot_dir_end() || off + len > PAGE_SIZE
                                  → Err(Storage(format!("corrupt page: cell {idx} is out of bounds")))
Ok(Some(&buf[off..off + len]))
```

#### `heap::insert_row`

現状の処理のまま、次だけ変える。

- 戻り値を `Result<()>` にする。
- `.expect("fresh page must fit one row")`（2 か所）を `.ok_or_else(|| Error::Storage("row does not fit in an empty page".into()))?` にする。

#### `heap::scan`

```text
rows = []
page_id = table.first_page
visited = 0
while page_id != 0:
  visited += 1
  if visited > storage.page_count():
    return Err(Storage(format!("corrupt image: page chain of table '{}' does not terminate", table.name)))
  buf = read_page(page_id)?
  page = SlottedPage::new(&mut buf)
  for slot in 0..page.num_slots():
    if let Some(cell) = page.cell(slot)? { rows.push(decode_row(cell)?) }
  page_id = page.next()
Ok(rows)
```

#### パーサ（parser.rs）

```text
parse(sql):
  tokens = tokenize(sql)?                  // 字句エラーは従来どおり Parse
  先頭トークンが Word(w) で、w を大文字にしたものが
    CREATE / INSERT / UPDATE / DELETE / DROP / ALTER / REPLACE / TRUNCATE のいずれか
      → Err(ReadOnly(format!("{W} is not supported; leafdb is read-only and only accepts SELECT")))   // W は大文字化した語
  先頭が SELECT → parse_select（現状のまま）
  それ以外 → Err(Parse(format!("unsupported or unknown statement start: {:?}", 先頭トークン)))   // 空入力も含む（現状の文言を維持）
  末尾の ';' を 1 つ読み飛ばし、残りがあれば Err(Parse("unexpected trailing tokens after statement"))
```

`parse_create_table` / `parse_insert` / `parse_update` / `parse_delete` は削除する。式の解析は現状維持。

#### 実行器（executor.rs）

```text
query(catalog, storage, select, params):
  bind_params(&mut select, params)?        // filter 内の ? を出現順に置換。数が合わなければ Parameter（現状と同じ文言）
  schema = catalog.get(&select.table) なければ Catalog("no such table '{t}'")
  out_indices = projection から解決。未知の列は Catalog("no such column '{c}'")
  columns = out_indices に対応する列名
  rows = []
  for row in heap::scan(storage, schema)?:
    if filter があり、filter_matches(filter, &row, schema)? が false → continue
    projected = out_indices の各 i について row.get(i).cloned()
                 なければ Err(Storage("corrupt row: fewer values than the table schema"))
    rows.push(projected)
  Ok(QueryResult { columns, rows })
```

- `eval_predicate` の `Expr::Column` も `row.get(idx)` を使い、無ければ上と同じ `Storage` エラーにする。
- `eval_binop` は `unreachable!` を使わない形にする。

```rust
fn eval_binop(op: BinOp, l: Value, r: Value) -> Value {
    match op {
        BinOp::And => Value::Boolean(truthy(&l) && truthy(&r)),
        BinOp::Or => Value::Boolean(truthy(&l) || truthy(&r)),
        BinOp::Eq => compare(&l, &r, |o| o == Ordering::Equal),
        BinOp::NotEq => compare(&l, &r, |o| o != Ordering::Equal),
        BinOp::Lt => compare(&l, &r, |o| o == Ordering::Less),
        BinOp::LtEq => compare(&l, &r, |o| o != Ordering::Greater),
        BinOp::Gt => compare(&l, &r, |o| o == Ordering::Greater),
        BinOp::GtEq => compare(&l, &r, |o| o != Ordering::Less),
    }
}
fn compare(l: &Value, r: &Value, f: impl Fn(Ordering) -> bool) -> Value {
    match compare_values(l, r) {
        None => Value::Null,
        Some(o) => Value::Boolean(f(o)),
    }
}
```

`compare_values`、`truthy`、`filter_matches` は現状維持。

#### 一括ロード（load.rs）

```text
RESERVED = ["SELECT", "FROM", "WHERE", "AND", "OR", "TRUE", "FALSE", "NULL"]

is_valid_identifier(s):
  s が空 → false
  先頭文字が is_alphabetic() または '_' でない → false
  残りの文字がすべて is_alphanumeric() または '_' でない → false
  s を大文字化したものが RESERVED に含まれる → false
  true

build(tables):
  tables が空 → Err(Catalog("at least one table is required"))
  storage = MemoryStorage::new()
  storage.allocate_page()?                  // ページ 0 = カタログ。最後に書き込む
  catalog = Catalog::new()
  for t in tables:
    !is_valid_identifier(&t.name)  → Err(Catalog(format!("invalid table name '{}'", t.name)))
    catalog.contains(&t.name)      → Err(Catalog(format!("duplicate table name '{}'", t.name)))
    t.columns が空                  → Err(Catalog(format!("table '{}' must have at least one column", t.name)))
    各列 c について（出現順）:
      !is_valid_identifier(&c.name) → Err(Catalog(format!("table '{}': invalid column name '{}'", t.name, c.name)))
      既出の列名と大文字小文字を無視して一致 → Err(Catalog(format!("table '{}': duplicate column name '{}'", t.name, c.name)))
    primary_key が true の列が 2 つ以上 → Err(Catalog(format!("table '{}': only a single-column PRIMARY KEY is supported", t.name)))
    schema = TableSchema { name: t.name, columns: t.columns, first_page: 0, last_page: 0 }
    pk = schema.primary_key_index()
    seen: HashSet<Vec<u8>> = 空
    for (i, row) in t.rows を列挙（i は 0 始まり）:
      row.len() != 列数 → Err(Type(format!("table '{}' row {i}: expected {列数} value(s), got {row.len()}", 表名)))
      values = 各 (j, v) について v.coerce_to(列 j の型)
               エラーは .context(format!("table '{}' row {i} column '{}'", 表名, 列名)) を付けて返す
      if let Some(p) = pk:
        values[p] が NULL → Err(Constraint(format!("table '{}' row {i}: primary key '{}' cannot be NULL", 表名, 列名)))
        key = values[p] を ByteWriter に Value::write したバイト列
        seen.insert(key) が false → Err(Constraint(format!("table '{}' row {i}: duplicate primary key value in column '{}'", 表名, 列名)))
      heap::insert_row(&mut storage, &mut schema, &values)
        エラーは .context(format!("table '{}' row {i}", 表名)) を付けて返す
    catalog.add_table(schema)?
  storage.write_page(0, &catalog.to_page()?)?      // テーブルが多すぎる場合の Catalog エラーはそのまま返す
  Ok((storage, catalog))
```

- 行は入力順に格納する（並べ替えない）。`SELECT` の結果も入力順になる。
- 主キーの無いテーブルでは重複値を許す。

#### `Database`（database.rs）

```text
load(tables):
  (storage, catalog) = load::build(tables)?
  Ok(Database { storage, catalog })

open_image(image):
  storage = MemoryStorage::from_image(image)?                   // 長さがページサイズの倍数でなければ Storage（現状維持）
  storage.page_count() == 0 → Err(Storage("database image is empty"))
  catalog = Catalog::from_page(ページ 0)?
  page_count = storage.page_count()
  各テーブル t について:
    (t.first_page == 0) != (t.last_page == 0)
      → Err(Storage(format!("corrupt image: table '{}' has an inconsistent page chain", t.name)))
    t.first_page >= page_count || t.last_page >= page_count
      → Err(Storage(format!("corrupt image: table '{}' points outside the image", t.name)))
    for row in heap::scan(&storage, t)?:
      row.len() != t.columns.len()
        → Err(Storage(format!("corrupt image: table '{}' has a row with {} value(s), expected {}", ...)))
      各値 v と列 c について、v が NULL でなく v.data_type() != Some(c.data_type)
        → Err(Storage(format!("corrupt image: table '{}' column '{}' has a value of the wrong type", ...)))
  Ok(Database { storage, catalog })

query(&self, sql, params):
  select = sql::parse(sql)?
  executor::query(&self.catalog, &self.storage, select, params)

export_image / tables: 現状維持
```

`open_image` で全行を検証しておくことで、以降の `query` がイメージの破損に起因して panic しないことを保証する。

#### `lib.rs`

- `pub mod load;` を追加する。
- クレートのドキュメントを「一括ロードで組み立て、`SELECT` だけを受け付ける読み取り専用エンジン。イメージは JavaScript 側で OPFS にキャッシュする」という内容に更新する。
- doc テストの例を `Database::load` + `query` に置き換える（todos テーブル 1 行を読み込み、`SELECT * FROM todos WHERE done = ?` で 1 行返ることを assert）。doc テスト内の `unwrap` / `panic!` は可。

### エラーケースと扱い

| ケース | エラー | 備考 |
| --- | --- | --- |
| 書き込み系の文 | `ReadOnly` | メッセージに大文字のキーワードを含む |
| 構文エラー・複数文 | `Parse` | |
| 未知のテーブル・列 | `Catalog` | |
| パラメータ数の不一致 | `Parameter` | |
| 一括ロードの定義不正（空、名前不正、重複、PK 複数） | `Catalog` | |
| 行の値の数の不一致・型の不一致 | `Type` | 表名・行番号（・列名）を含む |
| 主キーの NULL・重複 | `Constraint` | 表名・行番号を含む |
| 行がページに収まらない | `Storage` | `ensure_fits` のメッセージに表名・行番号を付ける |
| イメージの破損（長さ、MAGIC、バージョン、循環、範囲外、行の形） | `Storage` | panic・abort せずに返す |

ライブラリのコード（テスト以外）には `unwrap` / `expect` / `panic!` / `unreachable!` / スライスの無検査インデックスで入力由来の panic を起こし得るものを残さない。`debug_assert!` は残してよい。

### テスト観点（`tests/engine.rs`）

各グループはケースの配列（`&[(名前, 入力..., 期待値)]`）を 1 つの `#[test]` 関数内でループし、失敗時にケース名が分かるよう `assert!(..., "{name}")` の形で書く。ヘルパーとして `col(name, type, pk) -> Column`、`table(name, cols, rows) -> TableData` を用意してよい。

**`load_rejects_invalid_input`**（期待するバリアントとメッセージの部分文字列を確認）

| ケース | 期待 |
| --- | --- |
| テーブルのリストが空 | `Catalog`, "at least one table" |
| テーブル名 `1abc` | `Catalog`, "invalid table name" |
| テーブル名 `my table` | `Catalog`, "invalid table name" |
| テーブル名 `select`（予約語） | `Catalog`, "invalid table name" |
| テーブル名 `t` と `T` | `Catalog`, "duplicate table name" |
| 列が 0 個 | `Catalog`, "at least one column" |
| 列名が空文字 | `Catalog`, "invalid column name" |
| 列名 `null`（予約語） | `Catalog`, "invalid column name" |
| 列名 `a` と `A` | `Catalog`, "duplicate column name" |
| 主キー列が 2 つ | `Catalog`, "single-column PRIMARY KEY" |
| 3 列のテーブルで行 1 が 2 値 | `Type`, "row 1: expected 3 value(s), got 2" |
| INTEGER の主キー列に `Text` | `Type`, "row 0 column 'id'" |
| 主キーが NULL | `Constraint`, "cannot be NULL" |
| 行 2 が行 0 と同じ主キー | `Constraint`, "row 2: duplicate primary key" |
| 5000 文字の TEXT を持つ行 | `Storage`, "exceeds maximum cell size" かつ "row 0" |

**`load_accepts_valid_input`**

| ケース | 確認内容 |
| --- | --- |
| todos 3 行 | `SELECT *` の列名が `["id","title","done"]`、行が入力順 |
| BOOLEAN 列に `Integer(0)` / `Integer(1)` | `Boolean(false)` / `Boolean(true)` として返る |
| 主キー以外の列に NULL | そのまま NULL が返る |
| 行が 0 個のテーブル | `SELECT *` が 0 行、`tables()` に含まれる |
| 500 行（各行 20 文字以上の TEXT） | 全行が入力順に返り、`export_image().len() > 2 * PAGE_SIZE` |
| 2 テーブル | それぞれ独立に検索できる |
| 主キー無しのテーブルに同じ値の行 | 両方とも返る |

**`select_queries`**（`nums(id INTEGER PK, n INTEGER, flag BOOLEAN)`、id = 1..=10、n = id * 10、flag = id が偶数）

| SQL | パラメータ | 期待 |
| --- | --- | --- |
| `SELECT id FROM nums WHERE n > 30 AND n <= 60` | なし | `[4,5,6]` |
| `SELECT id FROM nums WHERE id = 1 OR id = 9` | なし | `[1,9]` |
| `SELECT id FROM nums WHERE (id = 1 OR id = 2) AND n = 20` | なし | `[2]` |
| `SELECT id FROM nums WHERE n != 10 AND n <> 20 AND n < 50` | なし | `[3,4]` |
| `SELECT id FROM nums WHERE n >= ? AND flag = ?` | `[Integer(80), Boolean(true)]` | `[8,10]` |
| `SELECT n, id FROM nums WHERE id = 3` | なし | `[[30,3]]`（列順どおり） |
| `select id from nums where id = 2` | なし | `[2]` |
| `SELECT id FROM nums WHERE id = 2;` | なし | `[2]` |
| `SELECT id FROM nums WHERE n = NULL` | なし | 0 行 |

**`select_errors`**

| SQL | パラメータ | 期待 |
| --- | --- | --- |
| `CREATE TABLE x (id INTEGER)` | なし | `ReadOnly`, "CREATE" |
| `insert into nums values (1)` | なし | `ReadOnly`, "INSERT" |
| `UPDATE nums SET n = 1` | なし | `ReadOnly`, "UPDATE" |
| `DELETE FROM nums` | なし | `ReadOnly`, "DELETE" |
| `DROP TABLE nums` | なし | `ReadOnly`, "DROP" |
| `ALTER TABLE nums` | なし | `ReadOnly`, "ALTER" |
| `SELECT * FROM missing` | なし | `Catalog` |
| `SELECT nope FROM nums` | なし | `Catalog` |
| `SELEC * FROM nums` | なし | `Parse` |
| `SELECT * FROM nums; SELECT * FROM nums` | なし | `Parse` |
| `SELECT id FROM nums WHERE id = ?` | なし | `Parameter` |
| `SELECT id FROM nums WHERE id = ?` | 2 個 | `Parameter` |

`ReadOnly` の `to_string()` が `"read-only: "` で始まることも 1 ケースで確認する。

**`image_round_trip`**: `load` → `export_image` → `open_image` で、`tables()` と `select_queries` の 1 ケース以上が元と同じ結果になる。

**`open_image_rejects_corrupt_images`**（各ケースで `Storage` が返り、panic しないこと）

画像はいずれも「`t(id INTEGER PK, s TEXT)` に 3 行」を `load` して `export_image` したものを改変する。ページ 0 がカタログ、ページ 1 が t の最初の（唯一の）ヒープページになる。

| ケース | 改変 | 期待メッセージ |
| --- | --- | --- |
| 空 | `vec![]` | "empty" |
| 長さ不正 | 先頭 100 バイトだけ | "multiple of page size" |
| MAGIC 不正 | ページ 0 の先頭 4 バイトを 0 に | "magic" |
| バージョン不正 | ページ 0 のバイト 4..8 を `99u32` の LE に | "format version" |
| 循環 | ページ 1 を `SlottedPage::new` で開き `set_next(1)` | "does not terminate" |
| セルが範囲外 | ページ 1 のスロット 0 の長さ（ページ先頭から 8+2..8+4 バイト）を `0xFFFFu16` の LE に | "out of bounds" |
| 値の数が巨大 | スロット 0 のセルの先頭 4 バイト（行の値の数）を `u32::MAX` の LE に | （`Storage` であれば文言は問わない） |

セルの位置はスロット 0 の offset（ページ先頭から 8..10 バイト、u16 LE）から読む。

**`bytes.rs` のユニットテスト**: `ByteReader::new(&[0xFF, 0xFF, 0xFF, 0xFF]).bytes()` が `Err(Storage)` を返す。

### 完了条件（単位 1）

- `cargo fmt --all -- --check`
- `cargo clippy -p leafdb-core --all-targets -- -D warnings`
- `cargo test -p leafdb-core`（上記テストと doc テストがすべて通る）

---

## 単位 2: `crates/leafdb-wasm` と `web/`

### 変更するファイル

| ファイル | 変更 |
| --- | --- |
| `crates/leafdb-wasm/src/lib.rs` | 読み取り専用 API に置き換え |
| `web/leafdb.js` | キャッシュ付きの `open`、`fromTables`、同期の `query` |
| `web/leafdb.d.ts` | 型定義を更新 |
| `web/index.html` | デモを読み取り専用・キャッシュのデモに置き換え |
| `README.md` | 読み取り専用の仕様に合わせて更新 |
| `docs/adr/0001-browser-first-rdb-in-rust-and-wasm.md` | Status 行のみ変更 |

### 公開する型・関数のシグネチャ（Rust）

```rust
#[wasm_bindgen]
pub struct LeafDb { inner: Database }

#[wasm_bindgen]
impl LeafDb {
    /// Builds a database from `{ [table]: { columns, rows } }`.
    #[wasm_bindgen(js_name = fromTables)]
    pub fn from_tables(tables: JsValue) -> Result<LeafDb, JsError>;

    /// Opens a database from a page image previously returned by `export`.
    #[wasm_bindgen(js_name = fromImage)]
    pub fn from_image(image: Uint8Array) -> Result<LeafDb, JsError>;

    /// Runs a SELECT and returns `{ columns: string[], rows: any[][] }`.
    pub fn query(&self, sql: &str, params: JsValue) -> Result<JsValue, JsError>;

    /// Serializes the database to a page image (for the OPFS cache).
    pub fn export(&self) -> Uint8Array;

    #[wasm_bindgen(js_name = tableNames)]
    pub fn table_names(&self) -> Array;
}
```

コンストラクタと `exec` は削除する。

### 処理の流れ（Rust）

```text
from_tables(tables):
  tables が null/undefined、オブジェクトでない、または配列
    → JsError("tables must be an object mapping table names to { columns, rows }")
  for [name, spec] in Object::entries(tables)（挿入順）:
    name = 文字列（entries のキーは常に文字列）
    spec がオブジェクトでない → JsError("table '{name}': definition must be an object")
    columns = Reflect::get(spec, "columns")。配列でない → JsError("table '{name}': columns must be an array")
    各列 c（インデックス k）:
      c がオブジェクトでない → JsError("table '{name}' column #{k}: must be an object")
      c.name が文字列でない → JsError("table '{name}' column #{k}: name must be a string")
      c.type が文字列でない → JsError("table '{name}' column '{cname}': type must be a string")
      DataType::from_keyword(type) が None → JsError("table '{name}' column '{cname}': unknown type '{type}'")
      c.primaryKey が undefined → false、真偽値 → その値、それ以外 → JsError("table '{name}' column '{cname}': primaryKey must be a boolean")
    rows = Reflect::get(spec, "rows")。配列でない → JsError("table '{name}': rows must be an array")
    各行 r（インデックス i）:
      Array::is_array(r) → 各要素 j を js_to_value。エラーの文脈は列 j があれば "table '{name}' row {i} column '{列名}'"、無ければ "table '{name}' row {i} value #{j}"
                            （値の数の不一致は core が Type エラーにする）
      r が null 以外のオブジェクト → 各列について Reflect::get(r, 列名)。undefined は NULL。列に無いキーは無視する
                            エラーの文脈は "table '{name}' row {i} column '{列名}'"
      それ以外 → JsError("table '{name}' row {i}: must be an array or an object")
  Database::load(Vec<TableData>) のエラーは to_js（to_string）で返す

js_to_value(v):
  null / undefined → Null
  as_bool → Boolean
  as_f64 → n:
    n が有限でない、n.fract() != 0.0、または n.abs() > 9007199254740991.0
      → Err("only safe integers between -(2^53 - 1) and 2^53 - 1 are supported")
    → Integer(n as i64)
  as_string → Text
  それ以外 → Err("unsupported value type (expected number, string, boolean, or null)")
  文脈付きのエラーは "{文脈}: {メッセージ}" の形の JsError にする

query(sql, params):
  params が null/undefined → []。配列でない → JsError("params must be an array")
  各要素を js_to_value。エラーの文脈は "parameter #{k}"（1 始まり）
  inner.query(sql, &params) の結果を { columns, rows } に変換（result_to_js は現状維持）

from_image(image): Database::open_image(image.to_vec())
export / table_names: 現状維持
```

`value_to_js` は現状維持（ロード時に安全な整数に制限しているため、返る整数は常に安全な範囲）。

### 公開する API（JavaScript: `web/leafdb.js`）

```js
/** Must equal leafdb_core::catalog::FORMAT_VERSION. */
const IMAGE_FORMAT = 1;

export class Database {
  /** Opens `name` at `version`, from the OPFS cache if present, otherwise via `load()`. */
  static async open({ name, version, load }) {}
  /** Builds a database without touching the cache. */
  static async fromTables(tables) {}
  /** Removes every cached image of `name`. */
  static async clearCache(name) {}
  /** True if the OPFS cache can be used in this context. */
  static get cacheAvailable() {}

  /** "cache" if opened from OPFS, "built" if built from load()/fromTables. */
  get source() {}
  /** Runs a SELECT; returns rows keyed by column name. */
  query(sql, params = []) {}
  /** Runs a SELECT; returns { columns, rows }. */
  queryRaw(sql, params = []) {}
  /** Names of all tables. */
  tables() {}
  /** Raw page image bytes. */
  exportImage() {}
}
export default Database;
```

`exec`、`persist`、`destroy`、`persistent` は削除する。`query` / `queryRaw` は同期関数にする。

### 処理の流れ（JavaScript）

```text
cacheFileName(name, version) = `${name}@${IMAGE_FORMAT}@${encodeURIComponent(version)}.leafdb`

open({ name, version, load }):
  name が文字列でない、または /^[A-Za-z0-9_-]+$/ に一致しない → throw TypeError("name must match /^[A-Za-z0-9_-]+$/")
  version が空でない文字列でない → throw TypeError("version must be a non-empty string")
  load が関数でない → throw TypeError("load must be a function")
  await ensureInit()
  file = cacheFileName(name, version)
  if cacheAvailable:
    bytes = await readFile(file)            // 無い・読めない場合は null
    if bytes:
      try: return new Database(LeafDb.fromImage(bytes), "cache")
      catch e: console.warn("leafdb: discarding unreadable cache", e); await removeFile(file)（失敗は無視）
  tables = await load()
  engine = LeafDb.fromTables(tables)        // エラーは呼び出し元へそのまま投げる
  if cacheAvailable:
    try: await writeFile(file, engine.export()); await removeCached(name, file)
    catch e: console.warn("leafdb: failed to write cache", e)
  return new Database(engine, "built")

removeCached(name, keep):
  root = await navigator.storage.getDirectory()
  for await (const entryName of root.keys()):
    entryName が `${name}@` で始まり ".leafdb" で終わり、keep と異なる → root.removeEntry(entryName)（個別の失敗は無視）

clearCache(name): name を open と同じ規則で検証し、cacheAvailable なら removeCached(name, null)

fromTables(tables): await ensureInit(); return new Database(LeafDb.fromTables(tables), "built")

query(sql, params): raw = engine.query(sql, params); raw.rows を列名キーのオブジェクトに変換して返す
```

`readFile` / `writeFile` / `removeFile` は現行の `readImage` / `writeImage` / `deleteImage` と同じ API（`getFileHandle`、`createWritable`、`removeEntry`）を使う。`cacheAvailable` は現行の `opfsAvailable()` と同じ判定。

### 型定義（`web/leafdb.d.ts`）

```ts
export type LeafValue = number | string | boolean | null;
export type Row = Record<string, LeafValue>;
export interface ResultSet { columns: string[]; rows: LeafValue[][]; }
export type ColumnType = "INTEGER" | "INT" | "TEXT" | "STRING" | "VARCHAR" | "BOOLEAN" | "BOOL";
export interface ColumnInput { name: string; type: ColumnType; primaryKey?: boolean; }
export interface TableInput { columns: ColumnInput[]; rows: Array<LeafValue[] | Record<string, unknown>>; }
export type TablesInput = Record<string, TableInput>;
export interface OpenOptions { name: string; version: string; load: () => TablesInput | Promise<TablesInput>; }

export class Database {
  static open(options: OpenOptions): Promise<Database>;
  static fromTables(tables: TablesInput): Promise<Database>;
  static clearCache(name: string): Promise<void>;
  static readonly cacheAvailable: boolean;
  readonly source: "cache" | "built";
  query(sql: string, params?: LeafValue[]): Row[];
  queryRaw(sql: string, params?: LeafValue[]): ResultSet;
  tables(): string[];
  exportImage(): Uint8Array;
}
export default Database;
```

各メンバーに現行と同程度の JSDoc を付ける。

### デモ（`web/index.html`）

見た目（CSS、色、パネル構成の雰囲気）は現行を踏襲し、内容を次に置き換える。Todo のパネルは削除する。

- **ヘッダーのバッジ**: `WASM: ready`、`OPFS cache: available` / `unavailable`、`source: cache` / `built`（`data-testid="badge-source"`）、`open: {ms} ms`（`data-testid="badge-open-ms"`）、`image: {n} bytes`、`tables: {names}`。
- **Dataset パネル**:
  - テキスト入力 `data-testid="data-version"`。初期値は `localStorage["leafdb-demo-version"]`、無ければ `"v1"`。
  - ボタン「Open」（`data-testid="open-db"`）: 入力値を localStorage に保存し、そのバージョンで開き直す。
  - ボタン「Clear cache」（`data-testid="clear-cache"`）: `Database.clearCache("leafdb-demo")` 後、状態表示を更新。
  - ボタン「Reload page」: `location.reload()`。
  - 表示 `data-testid="load-calls"`: このページ表示中に `load()` が呼ばれた回数。
  - ページ表示時に、入力値のバージョンで自動的に開く。
- **`load()` の内容**（API 呼び出しの模擬）: 300ms 待ってから次を返す。
  - `products`: 列 `id INTEGER PK`、`name TEXT`、`category TEXT`、`price INTEGER`、`in_stock BOOLEAN`。i = 0..4999 について行オブジェクト `{ id: i + 1, name: "Product " + (i + 1), category: ["books","games","music","tools","toys"][i % 5], price: (i * 37) % 9000 + 100, in_stock: i % 3 !== 0, updated_at: "2026-09-24" }`（`updated_at` はスキーマに無いキーで、無視されることを示す）。
  - `categories`: 列 `name TEXT PK`、`label TEXT`。5 行を配列形式で `["books","Books"]` のように。
- **SQL コンソール**:
  - テキストエリア（`data-testid="sql-input"`）の初期値は `SELECT id, name, price FROM products WHERE category = 'games' AND price < 1000`。
  - 「Run」ボタン（`data-testid="run-sql"`）と、例のボタン 3 つ（`SELECT * FROM categories`、`SELECT id, name FROM products WHERE in_stock = false AND price >= 9000`、`INSERT INTO products VALUES (1)`）。
  - 出力（`data-testid="sql-output"`）には、列名の行、先頭 50 行、`{総行数} row(s) in {ms} ms` を表示する。エラー時は `Error: {message}`。出力は `textContent` で設定する。
- **自己テストパネル**（`data-testid="run-suite"` / `data-testid="suite-output"`）: 次を順に確認し、PASS/FAIL を表示して `window.__leafdbSuite = { passed, log }` を設定する。
  1. `fromTables` で ADR-003 の成功条件の todos を作り、`SELECT * FROM todos WHERE done = ?`（`[false]`）が `{ id: 1, title: "Build a database", done: false }` の 1 行を返す。
  2. 主キーが重複した行を含む `fromTables` が例外になる。
  3. `2 ** 53` を含む行の `fromTables` が例外になる。
  4. `INSERT INTO todos VALUES (2, 'x', false)` が、メッセージに `read-only` を含む例外になる。
  5. キャッシュの往復: `clearCache("leafdb-selftest")` → `open({ name: "leafdb-selftest", version: "t1", load })` で `source === "built"`・`load` 呼び出し 1 回 → 同じ引数で再度 `open` して `source === "cache"`・`load` 呼び出し 1 回のまま・同じ検索結果 → `version: "t2"` で `source === "built"` → 最後に `clearCache("leafdb-selftest")`。`Database.cacheAvailable` が false の場合は、このチェックを `SKIP (OPFS unavailable)` と表示して PASS 扱いにする。

### README.md

- 冒頭の説明、Quick start、Architecture、Supported SQL、Status & roadmap を読み取り専用の仕様に書き換える。
  - Quick start は ADR-003 の成功条件のコード。
  - Architecture の図は `Database.open` → WASM API（`fromTables` / `fromImage` / `query`）→ Rust RDB Engine（一括ロード + SELECT）→ Storage、で OPFS はキャッシュとして横に置く。
  - Supported SQL は SELECT のみ（射影、`WHERE` の比較・`AND`・`OR`・括弧・`?`）。一括ロードで使える型（`INTEGER` / `TEXT` / `BOOLEAN` と別名、`NULL`、単一列の主キー）と、数値は安全な整数のみであることを書く。
  - ADR の一覧に ADR-003 を追加する。
- Building & testing は `make check` を先頭に置き、個別コマンドは現状の内容を残す。
- License は変更しない。

### ADR-001

`## Status` の本文 `Accepted` を `Accepted（一部を [ADR-003](0003-read-only-wasm-with-bulk-load.md) で置き換え）` に変える。それ以外は変更しない。

### エラーケースと扱い（単位 2）

| ケース | 扱い |
| --- | --- |
| `open` の引数不正 | `TypeError` を投げる |
| キャッシュが無い・読めない | `load()` から組み立てる。壊れたキャッシュは削除を試みる |
| キャッシュの書き込み・古いバージョンの削除の失敗 | `console.warn` のみ。`open` は成功させる |
| `load()` が投げた例外、`fromTables` の検証エラー | 呼び出し元へそのまま投げる |
| `query` のエラー（読み取り専用を含む） | 呼び出し元へそのまま投げる |

### テスト観点（単位 2）

自動テストは追加しない（対象外を参照）。`make check` で WASM のビルドが通ること、既存の core のテストが通ることを確認する。ブラウザでの動作は、親エージェントがデモページの自己テストと SQL コンソールで確認する。

### 完了条件（単位 2）

- `make check` が通る。
