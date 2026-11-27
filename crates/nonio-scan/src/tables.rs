//! Table reads in code, and the tables `.sql` schema files define. Text-based like the write adapters. Ported from
//! `facts/tables.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::collections::HashSet;
use std::sync::LazyLock;

use fancy_regex::Regex;
use nonio_facts::{FileFacts, TableDef};

use crate::adapters::Hit;
use crate::hash::hash_text;
use crate::re::{at, each, end, group, js, js_i, test};

const LIT: &str = r#"(?:"([^"\\\n]+)"|'([^'\\\n]+)'|`([^`\\$]+)`)"#;
/// Type arguments between a method name and its call: `db.get<RunRow>(`, `db.query<Pick<Row, "name">>(`.
const TARGS: &str = r"(?:<[^()]*?>\s*)?";

/// `db.get("t", id)`, `db.where("t", …)`, `db.all("t")`, `db.count("t")`.
static SCOUT: LazyLock<Regex> =
    LazyLock::new(|| js(&format!(r"(?<![\w$])db\s*\.\s*(?:get|where|all|count)\s*{TARGS}\(\s*{LIT}")));
/// `db.query(sql)`, `pool.query(sql)`, `sql.unsafe(sql)`, `knex.raw(sql)`: the first argument is SQL whatever its case.
static QUERY: LazyLock<Regex> = LazyLock::new(|| {
    js(&format!(
        r#"(?<![\w$])(?:db|pool|client|conn|connection|sql|knex)\s*\.\s*(?:query|execute|raw|unsafe)\s*{TARGS}\(\s*(["'`])"#
    ))
});
/// A string or template literal that starts like a read (upper-case keywords, so prose is skipped).
static SQL_LITERAL: LazyLock<Regex> = LazyLock::new(|| js(r#"(["'`])\s*(?:SELECT|WITH)\s"#));
/// Table names after FROM / JOIN in SQL text; `from (subquery)` and functions are skipped.
static FROM: LazyLock<Regex> =
    LazyLock::new(|| js_i(r#"\b(?:from|join)\s+(?:only\s+)?["`]?([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)?)["`]?(?!\s*\()"#));
/// Prisma: `prisma.user.findMany(…)`.
static PRISMA: LazyLock<Regex> = LazyLock::new(|| {
    js(
        r"(?<![\w$])(?:prisma|tx)\s*\.\s*([a-z][\w$]*)\s*\.\s*(?:findMany|findUnique|findUniqueOrThrow|findFirst|findFirstOrThrow|count|aggregate|groupBy)\s*\(",
    )
});
/// Drizzle: `db.select().from(users)`, `db.query.users.findMany(…)`.
static DRIZZLE_FROM: LazyLock<Regex> = LazyLock::new(|| js(r"\.\s*from\s*\(\s*([A-Za-z_$][\w$]*)\s*\)"));
static DRIZZLE_QUERY: LazyLock<Regex> =
    LazyLock::new(|| js(r"\bquery\s*\.\s*([A-Za-z_$][\w$]*)\s*\.\s*(?:findMany|findFirst)\s*\("));
/// Kysely: `db.selectFrom("t")`.
static KYSELY: LazyLock<Regex> = LazyLock::new(|| js(r#"\.\s*selectFrom\s*\(\s*(['"`])([\w.]+)\1"#));

static G_DB: LazyLock<Regex> = LazyLock::new(|| js(r"\bdb\s*\."));
static G_QUERY: LazyLock<Regex> = LazyLock::new(|| js(r"\.\s*(?:query|execute|raw|unsafe)\b"));
static G_SQL: LazyLock<Regex> = LazyLock::new(|| js(r"\b(?:SELECT|WITH)\s"));
static G_PRISMA: LazyLock<Regex> = LazyLock::new(|| js(r"\b(?:prisma|tx)\s*\."));
static G_SELECT: LazyLock<Regex> = LazyLock::new(|| js(r"\.\s*select\s*\("));
static G_QUERYDOT: LazyLock<Regex> = LazyLock::new(|| js(r"\bquery\s*\."));
static G_KYSELY: LazyLock<Regex> = LazyLock::new(|| js(r"\.\s*selectFrom\b"));

/// The literal starting at `open` (its quote), up to the matching close quote; templates may span lines.
fn literal_at(code: &str, open: usize) -> &str {
    let b = code.as_bytes();
    let q = b[open];
    let mut i = open + 1;
    while i < b.len() && b[i] != q {
        i += if b[i] == b'\\' { 2 } else { 1 };
    }
    crate::adapters::slice(code, open + 1, i)
}

const SQL_WORDS: [&str; 7] = ["select", "where", "lateral", "unnest", "generate_series", "jsonb_each", "json_each"];

fn sql_tables(sql: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in each(&FROM, sql) {
        let t = group(&m, 1).unwrap_or("").rsplit('.').next().unwrap_or("").to_string();
        if !SQL_WORDS.contains(&t.to_lowercase().as_str()) && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn lit(c: &fancy_regex::Captures<'_, str>) -> String {
    (1..4).find_map(|i| group(c, i)).unwrap_or_default().to_string()
}

/// Table reads in `code` (comments already blanked), in source order.
pub fn find_reads(code: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    let mut seen_sql = HashSet::new();
    if test(&G_DB, code) {
        out.extend(each(&SCOUT, code).map(|m| Hit { table: lit(&m), via: "db", at: at(&m) }));
    }
    if test(&G_QUERY, code) {
        for m in each(&QUERY, code) {
            let open = end(&m) - 1;
            seen_sql.insert(open);
            for table in sql_tables(literal_at(code, open)) {
                out.push(Hit { table, via: "sql", at: at(&m) });
            }
        }
    }
    if test(&G_SQL, code) {
        for m in each(&SQL_LITERAL, code) {
            if seen_sql.contains(&at(&m)) {
                continue;
            }
            for table in sql_tables(literal_at(code, at(&m))) {
                out.push(Hit { table, via: "sql", at: at(&m) });
            }
        }
    }
    let one = |re: &Regex, via: &'static str, g: usize, out: &mut Vec<Hit>| {
        out.extend(each(re, code).map(|m| Hit { table: group(&m, g).unwrap_or("").into(), via, at: at(&m) }));
    };
    if test(&G_PRISMA, code) {
        one(&PRISMA, "prisma", 1, &mut out);
    }
    if test(&G_SELECT, code) {
        one(&DRIZZLE_FROM, "drizzle", 1, &mut out);
    }
    if test(&G_QUERYDOT, code) {
        one(&DRIZZLE_QUERY, "drizzle", 1, &mut out);
    }
    if test(&G_KYSELY, code) {
        one(&KYSELY, "kysely", 2, &mut out);
    }
    out.sort_by_key(|h| h.at);
    out
}

static CONSTRAINT: LazyLock<Regex> =
    LazyLock::new(|| js_i(r"^(?:constraint|primary|foreign|unique|check|exclude|like|period)\b"));
static CREATE: LazyLock<Regex> = LazyLock::new(|| {
    js_i(
        r#"\bcreate\s+(?:or\s+replace\s+)?(?:(?:global|local)\s+)?(?:temp(?:orary)?\s+|unlogged\s+)?table\s+(?:if\s+not\s+exists\s+)?((?:[`"\[]?\w+[`"\]]?\.)?[`"\[]?\w+[`"\]]?)\s*\("#,
    )
});
static LINE_COMMENT: LazyLock<Regex> = LazyLock::new(|| js(r"--[^\n]*"));
static BLOCK_COMMENT: LazyLock<Regex> = LazyLock::new(|| js(r"/\*[\s\S]*?\*/"));
static COLUMN: LazyLock<Regex> = LazyLock::new(|| js(r#"^[`"\[]?([\w$]+)"#));

/// `text` with every match of `re` blanked byte for byte (newlines kept).
fn blank(text: &str, re: &Regex) -> String {
    let mut spans = Vec::new();
    for m in each(re, text) {
        spans.push((at(&m), end(&m)));
    }
    crate::adapters::mask_comments(text, &spans)
}

/// `CREATE TABLE [IF NOT EXISTS] name (cols…)` statements: table names (schema prefix dropped) and column names.
pub fn parse_sql_tables(sql: &str) -> Vec<TableDef> {
    let text = blank(&blank(sql, &LINE_COMMENT), &BLOCK_COMMENT);
    let b = text.as_bytes();
    let mut out = Vec::new();
    for m in each(&CREATE, &text) {
        let name: String = group(&m, 1).unwrap_or("").chars().filter(|c| !matches!(c, '`' | '"' | '[' | ']')).collect();
        let name = name.rsplit('.').next().unwrap_or("").to_string();
        let mut depth = 1;
        let mut i = end(&m);
        let mut parts = Vec::new();
        let mut from = i;
        while i < b.len() && depth > 0 {
            let c = b[i];
            if c == b'(' {
                depth += 1;
            } else if c == b')' {
                depth -= 1;
            } else if c == b'\'' {
                i += 1;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
            }
            if (c == b',' && depth == 1) || depth == 0 {
                parts.push(crate::adapters::slice(&text, from, i).to_string());
                from = i + 1;
            }
            i += 1;
        }
        let columns = parts
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty() && !test(&CONSTRAINT, p))
            .filter_map(|p| COLUMN.captures(p).ok().flatten().and_then(|c| group(&c, 1).map(str::to_string)))
            .collect();
        out.push(TableDef { name, columns });
    }
    out
}

/// Facts for a `.sql` schema file: the tables it defines (no symbols, calls or imports).
pub fn sql_facts(path: &str, source: &str) -> FileFacts {
    FileFacts { hash: hash_text(source), tables: parse_sql_tables(source), ..FileFacts::empty(path) }
}
