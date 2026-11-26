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
        if b[i] == b'"' {
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
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(last) = path.segments.last() {
            let span = last.ident.span().byte_range();
            self.references.push(json!({"name":last.ident.to_string(),"kind":"path","start":span.start,"end":span.end}));
        }
        syn::visit::visit_path(self, path)
    }
}
