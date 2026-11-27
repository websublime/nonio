//! Table reads and `.sql` schema tables. Ported from `packages/facts/test/tables.test.ts` of illuminate (MIT,
//! Copyright (c) 2026 johnz4021).

use nonio_scan::{extract_facts, find_reads, parse_sql_tables, ExtractContext};

fn tables(code: &str) -> Vec<String> {
    find_reads(code).into_iter().map(|r| format!("{}:{}", r.via, r.table)).collect()
}

fn no_ctx<T>(f: impl FnOnce(&ExtractContext<'_>) -> T) -> T {
    let resolve = |_: &str, _: &str| None;
    let record_of = |_: &str| None;
    f(&ExtractContext { resolve: &resolve, record_of: &record_of, entries: &[], routes: &[], nuxt: None })
}

#[test]
fn finds_scout_reads_with_literal_table_names_type_arguments_included() {
    assert_eq!(
        tables("const r = await db.get<RunRow>(\"runs\", id);\nreturn db.where<ContactRow>(\"contacts\", { email }, { limit: 1 });\ndb.all('cost_log');"),
        ["db:runs", "db:contacts", "db:cost_log"]
    );
    assert!(tables("db.insert(\"runs\", row); db.update(\"runs\", id, x);").is_empty());
    // writes are not reads
}

#[test]
fn reads_from_join_tables_out_of_db_query_sql_in_any_case_across_template_lines() {
    let code = "await db.query<Pick<ContactRow, \"name\">>(\"select name from contacts where name <> ''\");\n".to_string()
        + "await db.query(`select n.*, 1 - (e.vector <=> $2) as score\n   from embeddings e\n   join notes n on n.id = e.note_id`, [a]);\n"
        + "await db.query(\"select * from (select 1) x\");";
    assert_eq!(tables(&code), ["sql:contacts", "sql:embeddings", "sql:notes"]);
}

#[test]
fn reads_upper_case_sql_literals_elsewhere_but_not_prose() {
    assert_eq!(tables("const q = `SELECT id FROM public.runs JOIN steps ON true`;"), ["sql:runs", "sql:steps"]);
    assert!(tables("const msg = \"select a contact from the list\";").is_empty());
}

#[test]
fn finds_orm_reads() {
    assert_eq!(tables("await prisma.user.findMany({}); await tx.post.count();"), ["prisma:user", "prisma:post"]);
    assert_eq!(
        tables("db.select().from(users).where(eq(users.id, 1)); db.query.posts.findFirst({});"),
        ["drizzle:users", "drizzle:posts"]
    );
    assert_eq!(tables("db.selectFrom(\"pets\").selectAll()"), ["kysely:pets"]);
}

#[test]
fn lands_in_file_facts_reads_with_the_enclosing_symbol() {
    let f = no_ctx(|c| {
        extract_facts(
            "src/outreach/crm-sync.ts",
            "import { db } from \"../shared/db\";\nexport async function contactByEmail(email: string) {\n  const [c] = await db.where(\"contacts\", { email });\n  return c;\n}\n",
            c,
        )
    });
    assert_eq!(f.reads.len(), 1);
    assert_eq!(
        (f.reads[0].table.as_str(), f.reads[0].symbol.as_deref(), f.reads[0].via.as_str()),
        ("contacts", Some("src/outreach/crm-sync.ts#contactByEmail"), "db")
    );
}

const SQL: &str = r#"-- Contacts. Owned by outreach.
create sequence if not exists contacts_id_seq;
create table if not exists contacts (
  id         text primary key default 'ct_' || lpad(nextval('contacts_id_seq')::text, 4, '0'),
  email      text not null unique,
  phone      text, -- plain text, see beat 1
  company    text not null,
  updated_at timestamptz not null default now()
);
CREATE TABLE "public"."outreach_log" (
  id text,
  contact_id text not null references contacts (id),
  status text check (status in ('drafted', 'sent')),
  PRIMARY KEY (id),
  CONSTRAINT fk FOREIGN KEY (contact_id) REFERENCES contacts (id)
);
/* create table ignored (x int); */
create index if not exists outreach_log_contact_idx on outreach_log (contact_id);"#;

#[test]
fn parses_create_table_names_and_columns_skipping_constraints_and_comments() {
    let t = parse_sql_tables(SQL);
    let got: Vec<(String, Vec<String>)> = t.into_iter().map(|t| (t.name, t.columns)).collect();
    assert_eq!(
        got,
        [
            ("contacts".to_string(), ["id", "email", "phone", "company", "updated_at"].map(String::from).to_vec()),
            ("outreach_log".to_string(), ["id", "contact_id", "status"].map(String::from).to_vec()),
        ]
    );
}

#[test]
fn extract_gives_a_schema_file_its_tables_and_nothing_else() {
    let f = no_ctx(|c| extract_facts("src/outreach/schema.sql", SQL, c));
    assert_eq!(f.tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["contacts", "outreach_log"]);
    assert!(
        f.imports.is_empty() && f.symbols.is_empty() && f.calls.is_empty() && f.writes.is_empty() && f.reads.is_empty()
    );
    assert!(f.hash.len() == 16 && f.hash.chars().all(|c| c.is_ascii_hexdigit()));
}
