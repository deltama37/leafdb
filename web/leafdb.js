// High-level JavaScript/TypeScript API for leafdb.
//
// This wraps the synchronous WebAssembly engine (`LeafDb`) with an async,
// ergonomic API and OPFS persistence. The engine itself never touches OPFS
// (ADR-0001/0002): here we simply persist the exported page image to an OPFS
// file after every mutation and reload it when the database is opened.

import init, { LeafDb } from "./pkg/leafdb.js";

let initPromise = null;

/** Loads and instantiates the WebAssembly module exactly once. */
async function ensureInit() {
  if (!initPromise) {
    initPromise = init();
  }
  await initPromise;
}

/** True when this context can persist to the Origin Private File System. */
function opfsAvailable() {
  return (
    typeof navigator !== "undefined" &&
    navigator.storage &&
    typeof navigator.storage.getDirectory === "function"
  );
}

function fileNameFor(name) {
  return `${name}.leafdb`;
}

async function readImage(name) {
  try {
    const root = await navigator.storage.getDirectory();
    const handle = await root.getFileHandle(fileNameFor(name));
    const file = await handle.getFile();
    if (file.size === 0) return null;
    return new Uint8Array(await file.arrayBuffer());
  } catch (_e) {
    // A missing file throws NotFoundError; treat as "no database yet".
    return null;
  }
}

async function writeImage(name, bytes) {
  const root = await navigator.storage.getDirectory();
  const handle = await root.getFileHandle(fileNameFor(name), { create: true });
  const writable = await handle.createWritable();
  await writable.write(bytes);
  await writable.close();
}

async function deleteImage(name) {
  try {
    const root = await navigator.storage.getDirectory();
    await root.removeEntry(fileNameFor(name));
  } catch (_e) {
    // Nothing to remove.
  }
}

/**
 * A leafdb database. Open with {@link Database.open}.
 *
 * ```js
 * const db = await Database.open("app");
 * await db.exec(`CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT, done BOOLEAN)`);
 * await db.exec("INSERT INTO todos VALUES (?, ?, ?)", [1, "Build a database", false]);
 * const rows = await db.query("SELECT * FROM todos WHERE done = ?", [false]);
 * ```
 */
export class Database {
  constructor(name, engine, persistent) {
    this._name = name;
    this._engine = engine;
    this._persistent = persistent;
  }

  /**
   * Opens (or creates) a named database. When OPFS is available the database
   * is loaded from and saved to `"<name>.leafdb"` in the origin's private
   * file system, so data survives page reloads.
   */
  static async open(name = "leafdb") {
    await ensureInit();
    const persistent = opfsAvailable();
    const image = persistent ? await readImage(name) : null;
    const engine = new LeafDb(image ?? undefined);
    return new Database(name, engine, persistent);
  }

  /** True if writes are being persisted to OPFS. */
  get persistent() {
    return this._persistent;
  }

  /**
   * Runs a DDL/DML statement (`CREATE`, `INSERT`, `UPDATE`, `DELETE`).
   * Returns the number of affected rows and persists the change.
   */
  async exec(sql, params = []) {
    const affected = this._engine.exec(sql, params);
    await this.persist();
    return affected;
  }

  /**
   * Runs a `SELECT` and returns an array of row objects keyed by column name.
   */
  async query(sql, params = []) {
    const result = this._engine.query(sql, params);
    return result.rows.map((row) => {
      const obj = {};
      result.columns.forEach((col, i) => {
        obj[col] = row[i];
      });
      return obj;
    });
  }

  /** Like {@link query} but returns `{ columns, rows }` with positional rows. */
  async queryRaw(sql, params = []) {
    return this._engine.query(sql, params);
  }

  /** Names of all tables currently defined. */
  tables() {
    return Array.from(this._engine.tableNames());
  }

  /** Forces the current page image to be written to OPFS. */
  async persist() {
    if (!this._persistent) return;
    await writeImage(this._name, this._engine.export());
  }

  /** Returns the raw page image (useful for export/download). */
  exportImage() {
    return this._engine.export();
  }

  /** Deletes the persisted OPFS image for this database. */
  async destroy() {
    if (this._persistent) await deleteImage(this._name);
  }
}

export default Database;
