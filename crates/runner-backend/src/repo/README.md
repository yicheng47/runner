# Repository layer

This directory maps Runner's Rust domain types to SQLite rows. It is a small repository/data-mapper layer built on `rusqlite` and `serde_rusqlite`, not a general ORM.

The layer deliberately provides only the conventions Runner needs: row structs, stable storage encodings, small SQL builders, CRUD statements, and conversions between storage rows and domain values. It does not provide a query language, lazy relationships, identity maps, change tracking, or generated migrations.

## Boundaries

The persistence path has three layers:

```text
model                    repo                         ops
domain and API types  ↔  SQLite row types + SQL  ←  product behavior
                                                    transactions
                                                    event emission
                                                    error shaping
```

- [`model.rs`](../model.rs) defines values used by the application and its external interfaces, such as `Role`, `Crew`, `Slot`, `Mission`, and `Session`.
- `repo/*.rs` owns the row shape and SQL for one persistent object. Repository functions accept a `&rusqlite::Connection` and return `rusqlite::Result`.
- [`ops/*.rs`](../ops) validates commands, acquires pooled connections, owns transaction boundaries, coordinates multiple repositories, translates database failures into product errors, and emits application events.
- [`migrations/*.sql`](../../migrations) is the source of truth for the SQLite schema.

Repository functions never acquire a connection from `AppCore`. A `rusqlite::Transaction` dereferences to `Connection`, so the same repository function works both inside and outside a caller-owned transaction.

## Module anatomy

A table-backed module normally contains:

1. A row struct whose field names match SQLite column names.
2. A `COLUMNS` list shared by reads and inserts.
3. Storage adapters for timestamps or JSON stored in `TEXT` columns.
4. Conversions between the row struct and the domain type when their shapes differ.
5. SQL functions over `&Connection`.
6. Focused repository tests against an in-memory database.

[`role.rs`](role.rs) is the complete example. The public domain type exposes useful collections:

```rust
pub struct Role {
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    // ...
}
```

SQLite stores those collections as nullable JSON text, so the repository has a storage-specific type:

```rust
#[derive(Serialize, Deserialize)]
pub struct RoleRow {
    #[serde(with = "crate::repo::serde::json_text_opt")]
    pub args_json: Option<Vec<String>>,
    #[serde(with = "crate::repo::serde::json_text_opt")]
    pub env_json: Option<HashMap<String, String>>,
    // ...
}
```

`From<RoleRow> for Role` and `From<&Role> for RoleRow` contain that boundary. Legacy `NULL` collection columns become empty domain collections when read; new writes serialize empty collections as `[]` and `{}` to preserve the established database format.

When the database and domain shapes already match, as with most of `Crew`, the row conversion is intentionally mechanical. Keeping a row type still makes the storage boundary explicit and gives `serde_rusqlite` a type whose names match the SQL columns.

## Column and statement convention

Each table module keeps one ordered column list:

```rust
pub const COLUMNS: &[&str] = &[
    "id",
    "handle",
    "display_name",
    // ...
];
```

The shared helpers in `repo::mod` use it to build common statement fragments:

```rust
let sql = format!("SELECT {} FROM roles WHERE id = ?1", select_list(COLUMNS));

conn.execute(
    &insert_sql("roles", COLUMNS),
    to_params_named(row).map_err(ser_err)?.to_slice().as_slice(),
)?;
```

This keeps the row struct, selected column names, and named insert parameters visibly paired. `qualified_select_list("sl", COLUMNS)` produces `sl.id, sl.crew_id, ...` for joins while retaining the bare SQLite result-column names expected by `serde_rusqlite::from_row`.

Updates stay explicit because writable fields often differ from the full row. `to_params_named_with_fields` selects named values for a partial or immutable-field-aware update; it does not decide which fields are writable.

## Storage formats

[`repo::serde`](serde.rs) pins formats that must remain compatible with existing databases:

| Rust value | SQLite representation | Adapter |
| --- | --- | --- |
| `bool` | `INTEGER`, `0` or `1` | `serde_rusqlite` |
| Serde enums | `TEXT` using the enum's Serde rename | `serde_rusqlite` |
| `Timestamp` | RFC3339 `TEXT` | `rfc3339` |
| `Option<Timestamp>` | RFC3339 `TEXT` or `NULL` | `rfc3339_opt` |
| Collections/objects | JSON serialized into `TEXT` | `json_text` |
| Optional collections/objects | JSON `TEXT` or `NULL` | `json_text_opt` |

The mapping contract tests in `repo::mod` verify the raw SQLite types and bytes, including historical timestamp spellings and legacy JSON rows. Changing an adapter is a database compatibility change even when the Rust types remain the same.

## Domain types, row types, and joined results

Do not make a domain type mirror storage details merely to simplify SQL. Use a row type when SQLite names or encodings differ, then convert at the repository boundary.

Likewise, a foreign-key field is only an identifier. `Slot.role_id` does not contain a `Role`. A caller that needs a hydrated crew member loads the slots and referenced roles, then constructs `SlotWithRole` in the operations layer. This avoids duplicating role columns in `slots` while letting one role be reused by multiple crews.

Join results should remain explicit. Do not use `#[serde(flatten)]` to deserialize multiple SQL tables into one row: duplicate column names become ambiguous and storage layout leaks into API serialization. Either assemble a higher-level DTO from table reads or select one row type plus explicitly aliased ancillary columns, as `slot::list_for_role_with_crew_name` does.

## Transactions, invariants, and events

Repository functions execute the requested SQL; they do not own multi-step product invariants. For example, changing a crew lead requires clearing the current lead and promoting the new one atomically. `ops::slot` owns that transaction and calls the small `repo::slot` statements within it.

The same boundary applies to application events. A repository write does not emit `role/changed`, `crew/changed`, or another `AppEvent`; the state-level operation emits the event only after the database work succeeds.

Not-found messages, validation errors, and constraint explanations also belong in `ops`. Repositories return the original `rusqlite::Result`, with `de_err` and `ser_err` translating only `serde_rusqlite` failures into the corresponding `rusqlite` error categories.

## Adding or changing persistence

When adding a table:

1. Add a numbered migration and register it in `db.rs`.
2. Add the domain type to `model.rs` if the value is part of the application or external API.
3. Add a repository module with its row type, `COLUMNS`, SQL functions, and repository tests.
4. Add storage adapters only when an existing adapter cannot preserve the required byte format.
5. Add operations that enforce invariants, own transactions, shape errors, and emit events.

When adding a column:

1. Add the migration.
2. Update the row struct and `COLUMNS` together.
3. Update domain conversions and explicit update field lists.
4. Update every handcrafted `SELECT`, `INSERT`, join, fixture, and raw test insert that depends on the row shape.
5. Test both populated and `NULL`/default cases, plus legacy stored values when the encoding is not new.

Prefer the simplest SQL that preserves the invariant. Add shared helpers only for statement shapes or encodings used consistently across tables; product-specific behavior stays in its table module or in `ops`.
