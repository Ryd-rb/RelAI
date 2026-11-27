use anyhow::{Result, bail, ensure};
use fluxr::structured::SourceDocument;
use serde_json::{Value, json};
use syn::visit::Visit;

/// Parse a single retained source buffer. Byte offsets always refer to that buffer.
pub fn index(language: &str, source: &str) -> Result<Value> {
    ensure!(source.len() <= 256 * 1024, "Source exceeds parser limit");
    let mut declarations = Vec::new();
    let mut references = Vec::new();
    let mut diagnostics = Vec::new();
    match language {
        "rust" => match syn::parse_file(source) {
            Ok(file) => {
                let mut v = RustIndex {
                    declarations: &mut declarations,
                    references: &mut references,
                };
                v.visit_file(&file);
            }
            Err(e) => {
                let span = e.span().byte_range();
                diagnostics
                    .push(json!({"message":e.to_string(),"start":span.start,"end":span.end}));
            }
        },
        "fluxr" => {
            match SourceDocument::parse(source) {
                Ok(doc) => {
                    if doc.has_native_items() {
                        diagnostics.push(json!({"message":"Native declarations require separate Rust analysis."}));
                    }
                    let tokens = tokens(source);
                    for f in &doc.source.functions {
                        if let Some(pair) = tokens
                            .windows(2)
                            .find(|p| matches!(p[0].1, "fn" | "pub") && p[1].1 == f.name)
                        {
                            let (idx, name) = pair[1];
                            declarations.push(json!({"name":name,"kind":"function","start":idx,"end":idx+name.len()}));
                        }
                    }
                    for pair in tokens.windows(2).filter(|p| p[0].1 == "call") {
                        let (idx, name) = pair[1];
                        references.push(
                            json!({"name":name,"kind":"call","start":idx,"end":idx+name.len()}),
                        );
                    }
                    if let Err(err) = doc.compile() {
                        diagnostics.push(json!({"message":err.to_string()}));
                    }
                }
                Err(e) => diagnostics.push(json!({"message":e.to_string()})),
            }
        }
        _ => bail!("Supported analyzers are rust and fluxr"),
    }
    Ok(
        json!({"symbols":declarations,"references":references,"diagnostics":diagnostics,"source_digest":crate::hex(source.as_bytes())}),
    )
}
// Resolve only project-relative paths in the retained inventory's slash namespace.
fn retained_relative(source: &str, hint: &str) -> Option<String> {
    if hint.is_empty()
        || hint.starts_with('/')
        || hint.ends_with('/')
        || hint.contains(['\\', ':', '\0'])
    {
        return None;
    }
    let mut parts: Vec<_> = source
        .rsplit_once('/')
        .map_or_else(Vec::new, |(parent, _)| parent.split('/').collect());
    for part in hint.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

pub(crate) fn graph_file_metadata(path: &str, source: &str) -> Value {
    let mut roots = std::collections::BTreeSet::new();
    let mut modules = Vec::new();
    if source.len() > 65536 {
        return json!({"roots": [], "modules": []});
    }
    if path.rsplit('/').next() == Some("Cargo.toml") {
        if let Ok(manifest) = toml::from_str::<toml::Table>(source) {
            let lib = manifest.get("lib").into_iter();
            let bins = manifest
                .get("bin")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten();
            for target in lib.chain(bins) {
                if let Some(root) = target
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .and_then(|hint| retained_relative(path, hint))
                {
                    roots.insert(root);
                }
            }
        }
    } else if path.ends_with(".rs")
        && let Ok(file) = syn::parse_file(source)
    {
        for item in file.items {
            let syn::Item::Mod(module) = item else {
                continue;
            };
            if module.content.is_some() || module.attrs.len() != 1 {
                continue;
            }
            let attr = &module.attrs[0];
            if !attr.path().is_ident("path") {
                continue;
            }
            let syn::Meta::NameValue(meta) = &attr.meta else {
                continue;
            };
            let syn::Expr::Lit(expr) = &meta.value else {
                continue;
            };
            let syn::Lit::Str(literal) = &expr.lit else {
                continue;
            };
            let hint = literal.value();
            let Some(target) = retained_relative(path, &hint) else {
                continue;
            };
            let span = module.ident.span().byte_range();
            modules.push(json!({"name":module.ident.to_string(),"hint":hint,"target":target,"start":span.start,"end":span.end}));
        }
    }
    json!({"roots":roots,"modules":modules})
}

fn tokens(s: &str) -> Vec<(usize, &str)> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if let Some(end) = raw_literal_end(b, i) {
            i = end;
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() {
                let c = b[i];
                i += 1;
                if c == b'\\' {
                    i = (i + 1).min(b.len())
                } else if c == b'"' {
                    break;
                }
            }
        } else {
            while i < b.len() && !b[i].is_ascii_whitespace() {
                i += 1
            }
        }
        out.push((start, &s[start..i]));
    }
    out
}
// Keep a raw literal opaque to the secondary scanner, including its delimiters.
fn raw_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes[start] != b'r' {
        return None;
    }
    let mut quote = start + 1;
    while bytes.get(quote) == Some(&b'#') {
        quote += 1;
    }
    if bytes.get(quote) != Some(&b'"') {
        return None;
    }
    let hashes = &bytes[start + 1..quote];
    let mut i = quote + 1;
    while i < bytes.len() {
        if bytes[i] == b'"' && bytes[i + 1..].starts_with(hashes) {
            return Some(i + 1 + hashes.len());
        }
        i += 1;
    }
    Some(bytes.len())
}

struct RustIndex<'a> {
    declarations: &'a mut Vec<Value>,
    references: &'a mut Vec<Value>,
}
impl RustIndex<'_> {
    fn declare(&mut self, id: &syn::Ident, kind: &str) {
        let p = id.span().byte_range();
        self.declarations
            .push(json!({"name":id.to_string(),"kind":kind,"start":p.start,"end":p.end}));
    }
}
impl<'ast> Visit<'ast> for RustIndex<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        use syn::Item;
        match item {
            Item::Fn(v) => self.declare(&v.sig.ident, "function"),
            Item::Struct(v) => self.declare(&v.ident, "struct"),
            Item::Enum(v) => self.declare(&v.ident, "enum"),
            Item::Trait(v) => self.declare(&v.ident, "trait"),
            Item::Type(v) => self.declare(&v.ident, "type"),
            Item::Const(v) => self.declare(&v.ident, "const"),
            Item::Static(v) => self.declare(&v.ident, "static"),
            Item::Mod(v) if v.content.is_none() && v.attrs.is_empty() => {
                self.declare(&v.ident, "external_module")
            }
            Item::Mod(v) => self.declare(&v.ident, "module"),
            _ => (),
        }
        syn::visit::visit_item(self, item)
    }
    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.declare(&item.sig.ident, "method");
        syn::visit::visit_impl_item_fn(self, item)
    }
    fn visit_expr_method_call(&mut self, expr: &'ast syn::ExprMethodCall) {
        let span = expr.method.span().byte_range();
        self.references.push(json!({"name":expr.method.to_string(),"kind":"method_call","start":span.start,"end":span.end}));
        syn::visit::visit_expr_method_call(self, expr);
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(last) = path.segments.last() {
            let span = last.ident.span().byte_range();
            self.references.push(json!({"name":last.ident.to_string(),"kind":"path","start":span.start,"end":span.end}));
        }
        syn::visit::visit_path(self, path)
    }
}
