//! Module resolution for one repo. Ported from `imports.ts` and `facts/resolver.ts` of illuminate (MIT, Copyright
//! (c) 2026 johnz4021): oxc-resolver for files on disk (tsconfig `paths`/`baseUrl` with `extends`, package
//! `exports`/`imports`, workspaces, `.js` → `.ts`, directory index), and tsconfig `paths` for files not on disk yet.
//!
//! One deliberate difference: the original links workspace packages that aren't installed under
//! `<root>/.illuminate/ws-modules` so their `exports` maps apply. nonio never writes to the repo, so the links are
//! virtual: a file system in front of oxc-resolver shows them under `<root>/.nonio/ws-modules` without creating them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use oxc_resolver::{
    FileMetadata, FileSystem, FileSystemOs, ResolveError, ResolveOptions, ResolverGeneric, TsconfigDiscovery,
};
use serde_json::Value;

/// Package name for a bare specifier (`react-dom/client` → `react-dom`), `node:x` as is; `None` for aliases like
/// `@/x`, `~/x`, `#x`.
pub fn package_name(spec: &str) -> Option<String> {
    if spec.starts_with("node:") {
        return Some(spec.to_string());
    }
    let parts: Vec<&str> = spec.split('/').collect();
    let name = if spec.starts_with('@') {
        if parts.len() >= 2 {
            format!("{}/{}", parts[0], parts[1])
        } else {
            String::new()
        }
    } else {
        parts[0].to_string()
    };
    let word = |s: &str| {
        let mut c = s.chars();
        c.next().is_some_and(|f| f.is_ascii_alphanumeric())
            && c.all(|x| x.is_ascii_alphanumeric() || x == '_' || x == '.' || x == '-')
    };
    let ok = match name.strip_prefix('@') {
        Some(scoped) => scoped.split_once('/').is_some_and(|(scope, n)| word(scope) && word(n)),
        None => word(&name),
    };
    ok.then_some(name)
}

/// A repo-relative path normalized: POSIX separators, no `./`, `.`/`..` resolved; `None` if it escapes the root.
pub fn normalize_path(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let path = path.replace('\\', "/");
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// Directory part of a repo-relative path (`""` for root files).
pub fn dirname(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

pub fn is_relative(spec: &str) -> bool {
    spec == "." || spec == ".." || spec.starts_with("./") || spec.starts_with("../")
}

/// First existing file for a repo path written the way an import would be: as is, swapped JS extension, `.ts`,
/// `.tsx`, `/index.ts`, `/index.tsx`.
pub fn resolve_path(path: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    let base = normalize_path(path).filter(|b| !b.is_empty())?;
    let mut candidates = vec![base.clone()];
    for (ext, swaps) in [(".js", &[".ts", ".tsx"][..]), (".jsx", &[".tsx"][..]), (".mjs", &[".mts"][..])] {
        if let Some(stem) = base.strip_suffix(ext) {
            candidates.extend(swaps.iter().map(|s| format!("{stem}{s}")));
            break;
        }
    }
    for suffix in [".ts", ".tsx", "/index.ts", "/index.tsx"] {
        candidates.push(format!("{base}{suffix}"));
    }
    candidates.into_iter().find(|c| exists(c))
}

/// Resolve a specifier imported by `from_file` to a repo path, `vendor:<pkg>` (a package, or `vendor:node:<x>` for a
/// builtin), or `None`. With a repo resolver, files on disk resolve through oxc-resolver, and files `exists` knows but
/// disk doesn't (another version of the repo) through relative paths and tsconfig `paths`. Without one, only
/// relative specifiers resolve.
pub fn resolve_specifier(
    from_file: &str,
    spec: &str,
    exists: &dyn Fn(&str) -> bool,
    repo: Option<&RepoResolver>,
) -> Option<String> {
    let from = normalize_path(from_file).unwrap_or_else(|| from_file.to_string());
    if is_relative(spec) {
        if let Some(hit) = repo.and_then(|r| r.resolve(&from, spec)) {
            if !hit.starts_with("vendor:") {
                return Some(hit);
            }
        }
        let dir = dirname(&from);
        return resolve_path(&if dir.is_empty() { spec.to_string() } else { format!("{dir}/{spec}") }, exists);
    }
    let repo = repo?;
    let hit = repo.resolve(&from, spec);
    if let Some(h) = &hit {
        if !h.starts_with("vendor:") {
            return hit;
        }
    }
    let (bases, alias) = repo.candidates(&from, spec);
    if let Some(p) = bases.iter().find_map(|b| resolve_path(b, exists)) {
        return Some(p);
    }
    if hit.is_some() {
        return hit;
    }
    if alias || spec.starts_with('#') {
        return None; // an alias or a package import, not a package
    }
    package_name(spec).map(|p| format!("vendor:{p}"))
}

/// Module resolution against one repo on disk.
pub struct RepoResolver {
    root: PathBuf,
    resolver: ResolverGeneric<Workspaces>,
    matchers: RefCell<HashMap<PathBuf, Option<std::rc::Rc<PathsMatcher>>>>,
}

impl RepoResolver {
    pub fn new(root: &Path) -> Self {
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let mut modules = vec!["node_modules".to_string()];
        let workspaces = Workspaces::new(&root);
        if !workspaces.links.is_empty() {
            modules.push(workspaces.shim.to_string_lossy().into_owned());
        }
        let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let options = ResolveOptions {
            tsconfig: Some(TsconfigDiscovery::Auto),
            extensions: strings(&[".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json", ".d.ts"]),
            extension_alias: vec![
                (".js".into(), strings(&[".ts", ".tsx", ".js", ".jsx"])),
                (".jsx".into(), strings(&[".tsx", ".jsx"])),
                (".mjs".into(), strings(&[".mts", ".mjs"])),
                (".cjs".into(), strings(&[".cts", ".cjs"])),
            ],
            condition_names: strings(&["source", "types", "import", "require", "module", "node", "default"]),
            main_fields: strings(&["types", "module", "main"]),
            builtin_modules: true,
            node_path: false,
            modules,
            ..ResolveOptions::default()
        };
        Self {
            resolver: ResolverGeneric::new_with_file_system(workspaces, options),
            root,
            matchers: RefCell::default(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn in_repo(&self, abs: &Path) -> Option<String> {
        let rel = abs.strip_prefix(&self.root).ok()?;
        if rel.as_os_str().is_empty() {
            return None;
        }
        let parts: Vec<String> = rel
            .components()
            .map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Option<_>>()?;
        Some(parts.join("/"))
    }

    /// Repo path, `vendor:<pkg>` (package or node builtin), or `None`. Files on disk only.
    pub fn resolve(&self, from_file: &str, spec: &str) -> Option<String> {
        match self.resolver.resolve_file(self.root.join(from_file), spec) {
            Err(ResolveError::Builtin { resolved, .. }) => Some(format!("vendor:{resolved}")),
            Err(_) => None,
            Ok(r) => {
                if let Some(rel) = self.in_repo(r.path()) {
                    if !rel.starts_with("node_modules/") && !rel.contains("/node_modules/") {
                        return Some(rel);
                    }
                }
                package_name(spec).map(|p| format!("vendor:{p}"))
            }
        }
    }

    /// Repo-relative candidate bases for a non-relative spec from tsconfig `paths`/`baseUrl` (for files that aren't on
    /// disk); `alias` is true when a `paths` pattern matched, i.e. the spec is not a package name.
    pub fn candidates(&self, from_file: &str, spec: &str) -> (Vec<String>, bool) {
        let dir = self.root.join(dirname(from_file));
        let matcher = self.matchers.borrow().get(&dir).cloned();
        let matcher = matcher.unwrap_or_else(|| {
            let m = PathsMatcher::find(&self.root, &dir).map(std::rc::Rc::new);
            self.matchers.borrow_mut().insert(dir.clone(), m.clone());
            m
        });
        let Some(m) = matcher else {
            return (Vec::new(), false);
        };
        let bases = m.bases(spec).iter().filter_map(|abs| self.in_repo(&normalize_abs(abs))).collect();
        (bases, m.patterns.iter().any(|(p, _)| matches_pattern(p, spec)))
    }
}

fn normalize_abs(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

fn matches_pattern(pattern: &str, spec: &str) -> bool {
    match pattern.find('*') {
        None => pattern == spec,
        Some(star) => {
            spec.len() + 1 >= pattern.len()
                && spec.starts_with(&pattern[..star])
                && spec.ends_with(&pattern[star + 1..])
        }
    }
}

/// tsconfig `paths` and `baseUrl` of the nearest tsconfig.json in the repo, through `extends`, the way get-tsconfig's
/// `createPathsMatcher` reads them.
pub struct PathsMatcher {
    /// Pattern and its substitutions, made absolute.
    patterns: Vec<(String, Vec<PathBuf>)>,
    base_url: Option<PathBuf>,
}

impl PathsMatcher {
    fn find(root: &Path, dir: &Path) -> Option<Self> {
        let mut at = dir.to_path_buf();
        let file = loop {
            let f = at.join("tsconfig.json");
            if f.is_file() {
                break f;
            }
            if at == root || !at.pop() || !at.starts_with(root) {
                return None;
            }
        };
        let mut options = Options::default();
        read_tsconfig(root, &file, &mut options, 0);
        let base_url = options.base_url;
        let paths = options.paths?;
        let patterns = paths
            .0
            .into_iter()
            .map(|(pattern, subs)| {
                let base = base_url.clone().unwrap_or_else(|| paths.1.clone());
                (pattern, subs.iter().map(|s| base.join(s)).collect())
            })
            .collect::<Vec<_>>();
        if patterns.is_empty() && base_url.is_none() {
            return None;
        }
        Some(Self { patterns, base_url })
    }

    fn bases(&self, spec: &str) -> Vec<PathBuf> {
        if is_relative(spec) {
            return Vec::new();
        }
        let exact = self.patterns.iter().find(|(p, _)| !p.contains('*') && p == spec);
        let best = exact.or_else(|| {
            self.patterns
                .iter()
                .filter(|(p, _)| p.contains('*') && matches_pattern(p, spec))
                .max_by_key(|(p, _)| p.find('*').unwrap_or(0))
        });
        match best {
            None => self.base_url.iter().map(|b| b.join(spec)).collect(),
            Some((p, subs)) => {
                let star = p.find('*').map(|i| &spec[i..spec.len() - (p.len() - i - 1)]).unwrap_or("");
                subs.iter().map(|s| PathBuf::from(s.to_string_lossy().replacen('*', star, 1))).collect()
            }
        }
    }
}

#[derive(Default)]
struct Options {
    base_url: Option<PathBuf>,
    /// The patterns, and the directory of the config that declared them (their base without `baseUrl`).
    paths: Option<(Vec<Pattern>, PathBuf)>,
}

/// A tsconfig `paths` entry: the pattern and its substitutions.
type Pattern = (String, Vec<String>);

/// Read `file` into `out`: what it extends first, then its own options over them.
fn read_tsconfig(root: &Path, file: &Path, out: &mut Options, depth: usize) {
    if depth > 8 {
        return;
    }
    let Some(json) = fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&jsonc(&t)).ok()) else {
        return;
    };
    let dir = file.parent().unwrap_or(root).to_path_buf();
    let extends: Vec<&str> = match &json["extends"] {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    for e in extends {
        let mut p = if e.starts_with('.') || Path::new(e).is_absolute() {
            dir.join(e)
        } else {
            root.join("node_modules").join(e)
        };
        if p.is_dir() {
            p = p.join("tsconfig.json");
        } else if p.extension().is_none_or(|x| x != "json") && !p.is_file() {
            p.set_extension("json");
        }
        read_tsconfig(root, &p, out, depth + 1);
    }
    let options = &json["compilerOptions"];
    if let Some(b) = options["baseUrl"].as_str() {
        out.base_url = Some(dir.join(b));
    }
    if let Some(paths) = options["paths"].as_object() {
        let list = paths
            .iter()
            .map(|(k, v)| {
                let subs = v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect());
                (k.clone(), subs.unwrap_or_default())
            })
            .collect();
        out.paths = Some((list, dir));
    }
}

/// JSON with comments and trailing commas (tsconfig) as plain JSON.
fn jsonc(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                out.push(b'"');
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        out.push(b[i]);
                        i += 1;
                    }
                    out.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    out.push(b'"');
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b',' => {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if !matches!(b.get(j), Some(b'}') | Some(b']')) {
                    out.push(b',');
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Workspace package dirs (repo-relative) from package.json `workspaces` and pnpm-workspace.yaml.
pub fn workspace_dirs(root: &Path) -> Vec<String> {
    let mut globs: Vec<String> = Vec::new();
    if let Some(pkg) =
        fs::read_to_string(root.join("package.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        let list = match &pkg["workspaces"] {
            Value::Array(a) => a.clone(),
            Value::Object(o) => o.get("packages").and_then(Value::as_array).cloned().unwrap_or_default(),
            _ => Vec::new(),
        };
        globs.extend(list.iter().filter_map(Value::as_str).map(String::from));
    }
    if let Ok(yaml) = fs::read_to_string(root.join("pnpm-workspace.yaml")) {
        globs.extend(pnpm_packages(&yaml));
    }
    let clean = |g: &str| g.trim_start_matches("./").trim_end_matches('/').to_string();
    let include: Vec<String> = globs.iter().filter(|g| !g.starts_with('!')).map(|g| clean(g)).collect();
    if include.is_empty() {
        return Vec::new();
    }
    let ignore: Vec<String> = globs.iter().filter_map(|g| g.strip_prefix('!')).map(clean).collect();
    let depth = include.iter().map(|g| if g.contains("**") { 4 } else { g.split('/').count() }).max().unwrap_or(1);
    let mut out = Vec::new();
    fn walk(root: &Path, dir: &str, d: usize, depth: usize, inc: &[String], ign: &[String], out: &mut Vec<String>) {
        if d > depth {
            return;
        }
        let Ok(entries) = fs::read_dir(root.join(dir)) else {
            return;
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "node_modules" && !n.starts_with('.'))
            .collect();
        names.sort();
        for name in names {
            let rel = if dir.is_empty() { name } else { format!("{dir}/{name}") };
            if nonio_facts::matches_any(inc, &rel)
                && !nonio_facts::matches_any(ign, &rel)
                && root.join(&rel).join("package.json").is_file()
            {
                out.push(rel.clone());
            }
            walk(root, &rel, d + 1, depth, inc, ign, out);
        }
    }
    walk(root, "", 1, depth, &include, &ignore, &mut out);
    out.sort();
    out
}

/// The `packages:` list of a pnpm-workspace.yaml (the only part read; a full YAML parser isn't needed for it).
fn pnpm_packages(yaml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in yaml.lines() {
        let t = line.trim_end();
        if !line.starts_with([' ', '\t', '-']) && !t.is_empty() {
            inside = t.trim() == "packages:";
            continue;
        }
        if inside {
            if let Some(item) = t.trim_start().strip_prefix('-') {
                let item = item.split(" #").next().unwrap_or("").trim().trim_matches(['\'', '"']);
                if !item.is_empty() {
                    out.push(item.to_string());
                }
            }
        }
    }
    out
}

/// The real file system, plus workspace packages that aren't installed, shown as links under `<root>/.nonio/ws-modules`
/// so that oxc-resolver applies their `exports` maps. Nothing is written to disk.
struct Workspaces {
    os: FileSystemOs,
    shim: PathBuf,
    /// `<shim>/<package name>` → the package's directory.
    links: Vec<(PathBuf, PathBuf)>,
}

enum At {
    Real(PathBuf),
    Link(PathBuf),
    Dir,
    Missing,
}

impl Workspaces {
    fn new(root: &Path) -> Self {
        let shim = root.join(".nonio").join("ws-modules");
        let mut links = Vec::new();
        for dir in workspace_dirs(root) {
            let name = fs::read_to_string(root.join(&dir).join("package.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v["name"].as_str().map(String::from));
            if let Some(name) = name {
                if !root.join("node_modules").join(&name).exists() {
                    links.push((shim.join(&name), root.join(&dir)));
                }
            }
        }
        Self { os: <FileSystemOs as FileSystem>::new(), shim, links }
    }

    fn at(&self, p: &Path) -> At {
        if !p.starts_with(&self.shim) {
            return At::Real(p.to_path_buf());
        }
        for (link, target) in &self.links {
            if p == link {
                return At::Link(target.clone());
            }
            if let Ok(rest) = p.strip_prefix(link) {
                return At::Real(target.join(rest));
            }
        }
        if self.links.iter().any(|(link, _)| link.starts_with(p)) {
            At::Dir
        } else {
            At::Missing
        }
    }
}

fn missing() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

impl FileSystem for Workspaces {
    fn new() -> Self {
        Self { os: <FileSystemOs as FileSystem>::new(), shim: PathBuf::new(), links: Vec::new() }
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.at(path) {
            At::Real(p) | At::Link(p) => self.os.read(&p),
            _ => Err(missing()),
        }
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        match self.at(path) {
            At::Real(p) | At::Link(p) => self.os.read_to_string(&p),
            _ => Err(missing()),
        }
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        match self.at(path) {
            At::Real(p) | At::Link(p) => self.os.metadata(&p),
            At::Dir => Ok(FileMetadata::new(false, true, false)),
            At::Missing => Err(missing()),
        }
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        match self.at(path) {
            At::Real(p) => self.os.symlink_metadata(&p),
            At::Link(_) => Ok(FileMetadata::new(false, false, true)),
            At::Dir => Ok(FileMetadata::new(false, true, false)),
            At::Missing => Err(missing()),
        }
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        match self.at(path) {
            At::Real(p) => self.os.read_link(&p),
            At::Link(target) => Ok(target),
            _ => Err(missing().into()),
        }
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        match self.at(path) {
            At::Real(p) | At::Link(p) => self.os.canonicalize(&p),
            At::Dir => Ok(path.to_path_buf()),
            At::Missing => Err(missing()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_packages() {
        assert_eq!(package_name("react-dom/client").as_deref(), Some("react-dom"));
        assert_eq!(package_name("@acme/core/lib").as_deref(), Some("@acme/core"));
        assert_eq!(package_name("node:fs").as_deref(), Some("node:fs"));
        assert_eq!(package_name("@/x"), None);
        assert_eq!(package_name("~/x"), None);
        assert_eq!(package_name("#x"), None);
    }

    #[test]
    fn reads_jsonc() {
        let v: Value = serde_json::from_str(&jsonc("{ // c\n \"a\": [1, 2,], /* x */ \"b\": \"//\", }")).unwrap();
        assert_eq!(v["b"], "//");
        assert_eq!(v["a"][1], 2);
    }

    #[test]
    fn reads_pnpm_packages() {
        assert_eq!(pnpm_packages("packages:\n  - 'apps/*'\n  - \"!apps/old\"\nother: 1\n"), ["apps/*", "!apps/old"]);
    }
}
