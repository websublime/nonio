//! Table writes found in comment-free code, by text so they need no AST. Ported from `facts/adapters.ts` of
//! illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::re::{at, each, end, group, js, test};

/// A table read or write found in the text, at a byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub table: String,
    pub via: &'static str,
    pub at: usize,
}

const LIT: &str = r#"(?:"([^"\\\n]+)"|'([^'\\\n]+)'|`([^`\\$]+)`)"#;

/// `db.insert("t", …)` / `db.update("t", …)` with a literal table name.
static SCOUT: LazyLock<Regex> = LazyLock::new(|| js(&format!(r"(?<![\w$])db\s*\.\s*(?:insert|update)\s*\(\s*{LIT}")));
/// Prisma: `prisma.user.create(…)`, `tx.post.updateMany(…)`.
static PRISMA: LazyLock<Regex> = LazyLock::new(|| {
    js(
        r"(?<![\w$])(?:prisma|tx)\s*\.\s*([a-z][\w$]*)\s*\.\s*(?:create|createMany|createManyAndReturn|update|updateMany|updateManyAndReturn|upsert|delete|deleteMany)\s*\(",
    )
});
/// Drizzle: `db.insert(users).values(…)`, `.update(users).set(…)`, `.delete(users).where(…)`.
static DRIZZLE: LazyLock<Regex> = LazyLock::new(|| {
    js(
        r"\.\s*(?:insert|update|delete)\s*\(\s*([A-Za-z_$][\w$]*)\s*\)\s*\.\s*(?:values|set|where|returning|onConflictDoNothing|onConflictDoUpdate)\s*\(",
    )
});
/// Knex: `knex("t")…insert(…)`, `trx("t").update(…)`, `knex.table("t").del()`.
static KNEX: LazyLock<Regex> =
    LazyLock::new(|| js(r#"(?<![\w$])(?:knex|trx)\s*(?:\.\s*(?:table|into|from)\s*)?\(\s*(['"`])([\w.]+)\1\s*\)"#));
static KNEX_WRITE: LazyLock<Regex> =
    LazyLock::new(|| js(r"^[^;]{0,400}?\.\s*(?:insert|update|del|delete|upsert|merge|increment|decrement)\s*\("));
/// Kysely: `db.insertInto("t")`, `.updateTable("t")`, `.deleteFrom("t")`.
static KYSELY: LazyLock<Regex> = LazyLock::new(|| {
    js(r#"\.\s*(?:insertInto|replaceInto|updateTable|deleteFrom|mergeInto)\s*\(\s*(['"`])([\w.]+)\1"#)
});
/// Raw SQL (upper-case keywords, so prose like "Delete from cart" is skipped) at the start of a string or template.
static SQL: LazyLock<Regex> = LazyLock::new(|| {
    js(
        r#"(['"`])\s*(?:(?:INSERT|REPLACE)\s+(?:OR\s+\w+\s+)?INTO|DELETE\s+FROM|MERGE\s+INTO|UPDATE)\s+[`"\[]?([A-Za-z_][\w.]*)[`"\]]?(\s+SET\b)?"#,
    )
});
static UPDATE_AT: LazyLock<Regex> = LazyLock::new(|| js(r"^\s*UPDATE"));

static G_DB: LazyLock<Regex> = LazyLock::new(|| js(r"\bdb\s*\."));
static G_PRISMA: LazyLock<Regex> = LazyLock::new(|| js(r"\b(?:prisma|tx)\s*\."));
static G_DRIZZLE: LazyLock<Regex> = LazyLock::new(|| js(r"\.\s*(?:insert|update|delete)\s*\("));
static G_KNEX: LazyLock<Regex> = LazyLock::new(|| js(r"\b(?:knex|trx)\b"));
static G_KYSELY: LazyLock<Regex> =
    LazyLock::new(|| js(r"\.\s*(?:insertInto|replaceInto|updateTable|deleteFrom|mergeInto)\b"));
static G_SQL: LazyLock<Regex> = LazyLock::new(|| js(r"\b(?:INSERT|REPLACE|DELETE|MERGE|UPDATE)\s"));

/// The first of groups `1..=3` that took part (a literal in any of its three quotes).
fn lit(c: &fancy_regex::Captures<'_, str>, from: usize) -> String {
    (from..from + 3).find_map(|i| group(c, i)).unwrap_or_default().to_string()
}

/// `text[from..to]`, with both ends clamped to the text and moved back to a char boundary.
pub(crate) fn slice(text: &str, from: usize, to: usize) -> &str {
    let fix = |mut i: usize| {
        i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (a, b) = (fix(from), fix(to));
    if a >= b {
        ""
    } else {
        &text[a..b]
    }
}

/// Table writes in `code` (comments already blanked), in source order.
pub fn find_writes(code: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    if test(&G_DB, code) {
        out.extend(each(&SCOUT, code).map(|m| Hit { table: lit(&m, 1), via: "db", at: at(&m) }));
    }
    if test(&G_PRISMA, code) {
        out.extend(each(&PRISMA, code).map(|m| Hit {
            table: group(&m, 1).unwrap_or("").into(),
            via: "prisma",
            at: at(&m),
        }));
    }
    if test(&G_DRIZZLE, code) {
        out.extend(each(&DRIZZLE, code).map(|m| Hit {
            table: group(&m, 1).unwrap_or("").into(),
            via: "drizzle",
            at: at(&m),
        }));
    }
    if test(&G_KNEX, code) {
        for m in each(&KNEX, code) {
            // 400 UTF-16 units in the original; bytes here, which only ever looks a little less far.
            let after = slice(code, end(&m), end(&m) + 400);
            if test(&KNEX_WRITE, after) {
                out.push(Hit { table: group(&m, 2).unwrap_or("").into(), via: "knex", at: at(&m) });
            }
        }
    }
    if test(&G_KYSELY, code) {
        out.extend(each(&KYSELY, code).map(|m| Hit {
            table: group(&m, 2).unwrap_or("").into(),
            via: "kysely",
            at: at(&m),
        }));
    }
    if test(&G_SQL, code) {
        for m in each(&SQL, code) {
            // UPDATE needs SET.
            if test(&UPDATE_AT, slice(code, at(&m) + 1, at(&m) + 12)) && group(&m, 3).is_none() {
                continue;
            }
            out.push(Hit { table: group(&m, 2).unwrap_or("").into(), via: "sql", at: at(&m) });
        }
    }
    out.sort_by_key(|h| h.at);
    out
}

/// `source` with the given comment spans blanked: every byte but newlines becomes a space, so byte offsets hold.
pub fn mask_comments(source: &str, comments: &[(usize, usize)]) -> String {
    if comments.is_empty() {
        return source.to_string();
    }
    let mut out = source.as_bytes().to_vec();
    for &(start, stop) in comments {
        for b in &mut out[start.min(source.len())..stop.min(source.len())] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    }
    // Only whole comments are blanked and every comment delimiter is ASCII, so this is still UTF-8.
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}
