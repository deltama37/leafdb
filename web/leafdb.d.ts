// Type definitions for the high-level leafdb JavaScript API (`leafdb.js`).

/** A value stored in or returned from leafdb. */
export type LeafValue = number | string | boolean | null;

/** A row keyed by column name. */
export type Row = Record<string, LeafValue>;

/** Positional result set as returned by {@link Database.queryRaw}. */
export interface ResultSet {
  columns: string[];
  rows: LeafValue[][];
}

/**
 * A leafdb database backed by WebAssembly, with OPFS persistence in the
 * browser.
 */
export class Database {
  /** Opens or creates a named database (persisted to `<name>.leafdb` in OPFS). */
  static open(name?: string): Promise<Database>;

  /** Whether writes are persisted to OPFS in this environment. */
  readonly persistent: boolean;

  /** Runs a DDL/DML statement; returns affected row count. */
  exec(sql: string, params?: LeafValue[]): Promise<number>;

  /** Runs a SELECT; returns rows keyed by column name. */
  query(sql: string, params?: LeafValue[]): Promise<Row[]>;

  /** Runs a SELECT; returns `{ columns, rows }` with positional rows. */
  queryRaw(sql: string, params?: LeafValue[]): Promise<ResultSet>;

  /** Names of all defined tables. */
  tables(): string[];

  /** Forces a write of the current image to OPFS. */
  persist(): Promise<void>;

  /** Returns the raw page image bytes. */
  exportImage(): Uint8Array;

  /** Deletes the persisted OPFS image for this database. */
  destroy(): Promise<void>;
}

export default Database;
