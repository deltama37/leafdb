# ADR-001: ブラウザ向け軽量RDBをRustとWebAssemblyで実装する

## Status

Accepted

## Context

RDBMSの内部構造を、自ら設計・実装することで理解したい。

学習対象には以下を含む。

* ページ管理
* レコード管理
* インデックス
* SQLの解析・実行
* トランザクション
* WAL
* クラッシュリカバリ

単なる学習用実装ではなく、実際に利用可能な用途を持たせたい。

一般的なサーバー環境ではSQLiteやPostgreSQLなど成熟したRDBMSが存在するため、新しい軽量RDBを実装する実用上の利点は小さい。

一方、ブラウザでは通常のファイルシステムやサーバー型RDBを直接利用できず、WebAssembly、OPFSなどブラウザ固有の実行・永続化環境が存在する。

そのため、ブラウザを第一の実行環境として設計された軽量な組み込みRDBには、学習用途だけでなく一定の実用可能性がある。

## Decision

RustでRDBエンジンを実装し、WebAssemblyとしてブラウザ上で動作させる。

主用途を**ブラウザ内で完結するローカルRDB**とする。

以下を設計上の基本方針とする。

* ブラウザを第一の実行環境とする
* RustでDBエンジンを実装する
* WebAssemblyとして実行する
* OPFSを主要な永続化先とする
* 単一プロセス・単一スレッドを基本とする
* 小さいバイナリサイズと高速な起動を重視する
* JavaScript / TypeScriptから容易に利用できるAPIを提供する
* SQLiteやPostgreSQLとの完全互換性は目指さない
* SQLは実用上必要な範囲から段階的に実装する

概念的には以下の構造とする。

```text
JavaScript / TypeScript
          │
          ▼
       WASM API
          │
          ▼
┌─────────────────────┐
│ Rust RDB Engine     │
│                     │
│ SQL Parser          │
│ Planner / Executor  │
│ Table / Index       │
│ Page Manager        │
│ Transaction         │
└──────────┬──────────┘
           │
           ▼
       Storage
       ├─ Memory
       └─ OPFS
```

DBエンジン自体はOPFSのAPIへ直接依存させず、ストレージ層を抽象化する。

## Scope

初期実装では以下を対象とする。

* テーブル作成
* 行の追加・取得・更新・削除
* 基本的なデータ型
* 主キー
* B+Treeインデックス
* ページ単位のストレージ管理
* 最小限のSQL
* OPFSへの永続化
* 基本的なトランザクション

以下は初期対象外とする。

* SQL標準への完全準拠
* SQLite互換
* PostgreSQL互換
* サーバーとしての利用
* 分散処理
* レプリケーション
* 高度なクエリ最適化
* 大規模な同時接続
* 複数スレッドによる並列実行

## Why Rust

Rustを採用する。

主な理由は以下。

* WebAssemblyを主要なコンパイル先として扱いやすい
* ページやバイト列など低レベルなデータ構造を扱いやすい
* メモリ表現を明示的に設計できる
* メモリ安全性を保ちながらストレージエンジンを実装できる
* ネイティブ環境でもDBエンジン単体をテストできる

## Why Browser-first

WebAssemblyを使うこと自体を目的とはしない。

ブラウザではサーバー型RDBを直接利用できないため、ローカルでSQLによる構造化データ管理を行えることに独自の価値がある。

想定用途として以下を考える。

* オフライン対応Webアプリ
* Local-firstアプリ
* ブラウザ内での大量データ管理
* データをサーバーへ送信したくないアプリ
* 開発ツール
* ブラウザ上で動作する試験・分析環境

## Alternatives

### SQLite WASM

成熟しており、機能・信頼性・性能では自作実装より優れている。

一方、本プロジェクトではRDBMS内部を自ら設計・実装することも主要な目的であるため採用しない。

SQLiteとの競争ではなく、ブラウザ用途に必要な機能へ範囲を限定することで、より小さく単純なRDBを目指す。

### IndexedDB

ブラウザ標準で利用できるため追加バイナリが不要という利点がある。

一方、RDBMSではなく、SQL・リレーション・インデックス・問い合わせ実行などを自ら設計するという目的を満たさない。

### サーバー側RDB

PostgreSQL、SQLite、Cloudflare D1などを利用する方が、多くのWebアプリでは実用的である。

本プロジェクトではサーバーへの通信を必要としないローカルDBを対象とするため、用途が異なる。

## Consequences

### Positive

* RDBMSの内部構造を実装を通して理解できる
* ブラウザだけで完結するRDBとして利用できる
* オフライン・Local-first用途へ応用できる
* ブラウザという制約を利用して実装範囲を小さくできる
* SQLiteやPostgreSQLとは異なる設計判断を試せる

### Negative

* SQLiteなど既存RDBより機能・性能・信頼性で劣る
* WebAssemblyとブラウザのI/O制約を考慮する必要がある
* OPFSなどブラウザ固有のAPIへの対応が必要になる
* SQLを独自実装するコストが大きい
* バイナリサイズと機能数のトレードオフが発生する

## Success Criteria

最初の実用可能なバージョンでは、ブラウザ内だけで以下が動作することを目標とする。

```sql
CREATE TABLE todos (
    id INTEGER PRIMARY KEY,
    title TEXT,
    done BOOLEAN
);

INSERT INTO todos
VALUES (1, 'Build a database', false);

SELECT *
FROM todos
WHERE done = false;
```

JavaScript / TypeScriptから以下のように利用できることを目指す。

```ts
const db = await Database.open("app");

await db.exec(`
  CREATE TABLE todos (
    id INTEGER PRIMARY KEY,
    title TEXT,
    done BOOLEAN
  )
`);

await db.exec(
  "INSERT INTO todos VALUES (?, ?, ?)",
  [1, "Build a database", false],
);

const rows = await db.query(
  "SELECT * FROM todos WHERE done = ?",
  [false],
);
```

ブラウザを再読み込みしてもOPFSからデータを復元できることを、最初の完成条件とする。

## Non-goals

本プロジェクトの目的はSQLiteを置き換えることではない。

優先順位は以下とする。

```text
RDBMS内部の理解
    ↓
ブラウザで実際に利用できること
    ↓
実装の単純さ
    ↓
バイナリサイズ・起動速度
    ↓
機能数
```

機能数やSQL互換性を増やすことよりも、ブラウザ環境に適した小さく理解可能なRDBを維持することを優先する。
