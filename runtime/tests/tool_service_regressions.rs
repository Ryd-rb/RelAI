use base64::{Engine, engine::general_purpose::STANDARD};
use fluxr_app_host::App;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::Path,
};

struct Fixture {
    _temp: tempfile::TempDir,
    app: App,
    project: Value,
    root: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        let mut app = App::open(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"),
            &temp.path().join("private"),
        )
        .unwrap();
        let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
        Self {
            _temp: temp,
            app,
            project,
            root,
        }
    }
    fn tool(&mut self, tool: &str, arguments: Value) -> anyhow::Result<Value> {
        Ok(self.app.invoke(
            "tool_run",
            json!({"project":self.project,"tool":tool,"arguments":arguments}),
        )?["value"]
            .clone())
    }
    fn bytes(&mut self, artifact: &Value) -> Vec<u8> {
        STANDARD
            .decode(
                self.tool("artifact.read", json!({"artifact":artifact["id"]}))
                    .unwrap()["base64"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap()
    }
}

#[test]
fn null_filters_object_rows_required_numbers_and_query_cursors() {
    let mut f = Fixture::new();
    assert_eq!(
        f.tool(
            "data.transform",
            json!({"rows":[{}, {"x":null}, {"x":1}],"filter":{"field":"x","value":null}})
        )
        .unwrap(),
        json!([{"x":null}])
    );
    let error = f
        .tool("data.transform", json!({"rows":[{},1,"text"]}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("rows[1]"), "{error}");
    for fields in [json!("x"), json!({"field":"x"}), json!(["x", 1])] {
        let error = f
            .tool(
                "data.transform",
                json!({"rows":[{"x":null}],"fields":fields}),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("fields"), "{error}");
    }
    let error = f
        .tool(
            "data.transform",
            json!({"rows":[{"x":null}],"filter":{"field":"x"}}),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("value"), "{error}");
    for values in [Value::Null, json!({}), json!([1, "x"])] {
        assert!(f.tool("data.summarize", json!({"values":values})).is_err());
    }
    assert!(f.tool("data.summarize", json!({})).is_err());
    assert_eq!(
        f.tool("data.summarize", json!({"values":[]})).unwrap(),
        json!({"count":0,"sum":0,"min":null,"max":null,"mean":null})
    );
    assert!(
        !f.tool("data.read_query", json!({"template":"recent_events"}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    for after in [json!(-1), json!("0"), json!(1.5), Value::Null] {
        assert!(
            f.tool(
                "data.read_query",
                json!({"template":"recent_events","after":after})
            )
            .is_err()
        );
    }
    assert!(
        f.tool("data.read_query", json!({"template":"request_jobs"}))
            .is_err()
    );
}

#[test]
fn notebook_unchanged_text_preserves_execution_and_real_edits_clear_it() {
    let mut f = Fixture::new();
    let original = json!({"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[{"cell_type":"code","id":"stable","metadata":{"keep":true},"source":["one()\n","two()"],"execution_count":8,"outputs":[{"output_type":"stream","name":"stdout","text":["result"]}]}]});
    fs::write(
        f.root.join("input.ipynb"),
        serde_json::to_vec(&original).unwrap(),
    )
    .unwrap();
    for source in [
        json!("one()\ntwo()"),
        json!(["one()\n", "two()"]),
        json!(["one()", "\ntwo()"]),
    ] {
        let result = f
            .tool(
                "notebook.edit",
                json!({"path":"input.ipynb","edits":[{"index":0,"source":source}]}),
            )
            .unwrap();
        let edited: Value = serde_json::from_slice(&f.bytes(&result)).unwrap();
        assert_eq!(edited, original);
    }
    for source in [json!("changed()"), json!(["changed()\n", "again()"])] {
        let result = f
            .tool(
                "notebook.edit",
                json!({"path":"input.ipynb","edits":[{"index":0,"source":source}]}),
            )
            .unwrap();
        let edited: Value = serde_json::from_slice(&f.bytes(&result)).unwrap();
        assert_eq!(edited["cells"][0]["outputs"], json!([]));
        assert!(edited["cells"][0]["execution_count"].is_null());
        assert_eq!(edited["cells"][0]["id"], "stable");
        assert_eq!(
            edited["cells"][0]["metadata"],
            original["cells"][0]["metadata"]
        );
        assert!(
            edited["cells"][0]["source"]
                .as_array()
                .unwrap()
                .iter()
                .all(Value::is_string)
        );
    }
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(f.root.join("input.ipynb")).unwrap()).unwrap(),
        original
    );
}

#[test]
fn sheets_round_trip_reject_duplicates_and_preserve_exact_integer_policy() {
    let mut f = Fixture::new();
    let artifact = f.tool("sheet.write", json!({"sheets":[{"name":"Data","cells":[{"row":0,"column":0,"value":"text"},{"row":2,"column":3,"formula":"=1+2"}]}]})).unwrap();
    let read = f
        .tool("sheet.read", json!({"artifact":artifact["id"]}))
        .unwrap();
    let written = f.tool("sheet.write", read.clone()).unwrap();
    assert_eq!(
        f.tool("sheet.read", json!({"artifact":written["id"]}))
            .unwrap(),
        read
    );
    assert!(f.tool("sheet.write", json!({"sheets":[{"name":"Data","cells":[{"row":1,"column":2,"value":1},{"row":1,"column":2,"value":2}]}]})).unwrap_err().to_string().contains("Duplicate cell"));
    assert!(f.tool("sheet.write", json!({"sheets":[{"name":"Data","cells":[{"row":0,"column":0,"value":9007199254740993_u64}]}]})).unwrap_err().to_string().contains("text"));
}

#[test]
fn sparse_formula_coordinates_survive_without_cached_values() {
    let mut f = Fixture::new();
    let artifact = f.tool("sheet.write", json!({"sheets":[{"name":"Sparse","cells":[{"row":0,"column":0,"value":7},{"row":19,"column":9,"formula":"=1+2"}]}]})).unwrap();
    let bytes = f.bytes(&artifact);
    let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..input.len() {
        let mut entry = input.by_index(i).unwrap();
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if name == "xl/worksheets/sheet1.xml" {
            let xml = String::from_utf8(bytes).unwrap();
            assert!(xml.contains("<f>1+2</f><v>0</v>"));
            bytes = xml.replace("<f>1+2</f><v>0</v>", "<f>1+2</f>").into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    fs::write(
        f.root.join("sparse.xlsx"),
        output.finish().unwrap().into_inner(),
    )
    .unwrap();
    let read = f.tool("sheet.read", json!({"path":"sparse.xlsx"})).unwrap();
    let cell = read["sheets"][0]["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["row"] == 19 && c["column"] == 9)
        .unwrap();
    assert_eq!(cell["formula"], "1+2");
    assert!(cell["value"].is_null());
}

#[test]
fn sparse_sheet_writes_exact_representable_formula_caches() {
    let mut f = Fixture::new();
    let artifact = f
        .tool(
            "sheet.write",
            json!({"sheets":[{"name":"Sparse","cells":[
                {"row":99999,"column":0,"formula":"=21*2","value":42},
                {"row":99999,"column":1,"formula":"=1=1","value":true},
                {"row":99999,"column":2,"formula":"=\"Result\"","value":"Result"}
            ]}]}),
        )
        .unwrap();
    let read = f
        .tool("sheet.read", json!({"artifact":artifact["id"]}))
        .unwrap();
    let cells = read["sheets"][0]["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 3);
    assert_eq!(cells[0]["row"], 99999);
    assert_eq!(cells[0]["value"].as_f64(), Some(42.0));
    assert_eq!(cells[1]["value"], true);
    assert_eq!(cells[2]["value"], "Result");
    let error = f
        .tool(
            "sheet.write",
            json!({"sheets":[{"name":"Data","cells":[
                {"row":0,"column":0,"formula":"=\"42\"","value":"42"}
            ]}]}),
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("formula cache"));
    assert!(
        f.tool(
            "sheet.write",
            json!({"sheets":[{"name":"Data","cells":[
                {"row":0,"column":0,"value":1},
                {"row":99999,"column":0,"value":2}
            ]}]})
        )
        .unwrap_err()
        .to_string()
        .contains("Worksheet grid limit")
    );
}

#[test]
fn notebook_rejects_missing_required_metadata_and_code_fields() {
    let mut f = Fixture::new();
    let valid = json!({"nbformat":4,"nbformat_minor":2,"metadata":{},"cells":[
        {"cell_type":"code","metadata":{},"source":"pass","outputs":[],"execution_count":null}
    ]});
    for field in ["metadata", "nbformat_minor", "outputs", "execution_count"] {
        let mut malformed = valid.clone();
        if matches!(field, "outputs" | "execution_count") {
            malformed["cells"][0].as_object_mut().unwrap().remove(field);
        } else {
            malformed.as_object_mut().unwrap().remove(field);
        }
        fs::write(f.root.join("input.ipynb"), malformed.to_string()).unwrap();
        assert!(
            f.tool("notebook.edit", json!({"path":"input.ipynb","edits":[]}))
                .is_err(),
            "{field}"
        );
    }
    fs::write(f.root.join("input.ipynb"), valid.to_string()).unwrap();
    f.tool("notebook.edit", json!({"path":"input.ipynb","edits":[]}))
        .unwrap();
}

#[test]
fn archive_path_validation_rejects_conflicts_in_both_orders() {
    let mut f = Fixture::new();
    for (a, b, expected) in [
        ("notes", "notes/index.txt", "file/directory conflict"),
        ("Notes/a.txt", "notes/b.txt", "Inconsistent case"),
    ] {
        for (first, second) in [(a, b), (b, a)] {
            let error = f
                .tool(
                    "archive.create",
                    json!({"entries":[
                        {"path":first,"base64":"YQ=="},
                        {"path":second,"base64":"Yg=="}
                    ]}),
                )
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
        }
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.add_directory("notes/", options).unwrap();
    zip.start_file("notes/index.txt", options).unwrap();
    zip.write_all(b"index").unwrap();
    fs::write(
        f.root.join("explicit-directory.zip"),
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    f.tool("archive.list", json!({"path":"explicit-directory.zip"}))
        .unwrap();
}

#[test]
fn docx_empty_paragraph_cdata_and_real_tabs() {
    let mut f = Fixture::new();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "word/document.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>first</w:t></w:r></w:p><w:p/><w:p><w:r><w:t><![CDATA[a<&b]]></w:t></w:r></w:p></w:body></w:document>").unwrap();
    fs::write(
        f.root.join("input.docx"),
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let read = f
        .tool(
            "document.read",
            json!({"path":"input.docx","format":"docx"}),
        )
        .unwrap();
    assert_eq!(read, json!({"text":"first\n\na<&b\n","paragraphs":3}));
    let artifact = f
        .tool(
            "document.write",
            json!({"format":"docx","name":"   ","text":"a\tb\t"}),
        )
        .unwrap();
    assert_eq!(artifact["display_name"], "output.docx");
    let bytes = f.bytes(&artifact);
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert_eq!(xml.matches("<w:tab/>").count(), 2);
    assert_eq!(
        f.tool(
            "document.read",
            json!({"artifact":artifact["id"],"format":"docx"})
        )
        .unwrap()["text"],
        "a\tb\t\n"
    );
}
