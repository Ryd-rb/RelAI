//! Reusable bounded data codecs. These functions have no filesystem, application
//! database, routing, credential, process, or live-effect access.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{Cursor, Read, Write},
};

const INPUT: usize = 2 * 1024 * 1024;
const EXPANDED: usize = 8 * 1024 * 1024;
const ITEMS: usize = 4096;

fn text(value: &Value) -> Result<&str> {
    value.as_str().context("Expected text")
}
fn bounded(value: Value) -> Result<Value> {
    ensure!(
        serde_json::to_vec(&value)?.len() <= EXPANDED,
        "Decoded output exceeds limit"
    );
    Ok(value)
}
fn decoded(encoded: &Value) -> Result<Vec<u8>> {
    let encoded = text(encoded)?;
    ensure!(encoded.len() <= INPUT * 2, "Encoded input exceeds limit");
    let bytes = STANDARD.decode(encoded)?;
    ensure!(bytes.len() <= INPUT, "Input exceeds codec limit");
    Ok(bytes)
}

pub fn decode(format: &str, bytes: &[u8]) -> Result<Value> {
    ensure!(bytes.len() <= INPUT, "Input exceeds codec limit");
    bounded(match format {
        "markdown" | "text" => json!({"text":std::str::from_utf8(bytes)?}),
        "markdown.links" => {
            use pulldown_cmark::{Event, Options, Parser, Tag};
            let source = std::str::from_utf8(bytes)?;
            let mut links = Vec::new();
            for (event, span) in Parser::new_ext(source, Options::all()).into_offset_iter() {
                if let Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) =
                    event
                {
                    ensure!(links.len() < ITEMS, "Markdown link count limit");
                    let path = dest_url.split(['#', '?']).next().unwrap_or("");
                    let mut decoded = Vec::new();
                    let mut bytes = path.bytes();
                    while let Some(byte) = bytes.next() {
                        if byte == b'%' {
                            let high = bytes.next().context("Truncated percent escape")?;
                            let low = bytes.next().context("Truncated percent escape")?;
                            let high = (high as char)
                                .to_digit(16)
                                .context("Invalid percent escape")?;
                            let low = (low as char)
                                .to_digit(16)
                                .context("Invalid percent escape")?;
                            decoded.push((high * 16 + low) as u8);
                        } else {
                            decoded.push(byte);
                        }
                    }
                    links.push(json!({"target":dest_url.as_ref(),"path":String::from_utf8(decoded)?,"start":span.start,"end":span.end}));
                }
            }
            json!({"links":links})
        }
        "json" => serde_json::from_slice(bytes)?,
        "csv" => {
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .from_reader(bytes);
            let mut rows = Vec::new();
            let mut cells = 0;
            for record in reader.records() {
                let record = record?;
                cells += record.len();
                ensure!(rows.len() < ITEMS && cells <= ITEMS, "CSV row/cell limit");
                rows.push(record.iter().map(str::to_owned).collect::<Vec<_>>());
            }
            json!({"rows":rows})
        }
        "docx" => read_docx(bytes)?,
        "pdf" => read_pdf(bytes)?,
        "xlsx" => read_xlsx(bytes)?,
        "ipynb" => {
            let value: Value = serde_json::from_slice(bytes)?;
            validate_notebook(&value)?;
            value
        }
        "zip" => read_zip(bytes, true)?,
        "zip.index" => read_zip(bytes, false)?,
        _ => bail!("Unknown decode format {format}"),
    })
}

pub fn encode(format: &str, input: &Value) -> Result<Vec<u8>> {
    ensure!(
        serde_json::to_vec(input)?.len() <= EXPANDED,
        "Encode input exceeds limit"
    );
    let bytes = match format {
        "markdown" | "text" => text(&input["text"])?.as_bytes().to_vec(),
        "json" => serde_json::to_vec_pretty(input)?,
        "csv" => {
            let rows = input["rows"].as_array().context("Expected CSV rows")?;
            let mut writer = csv::WriterBuilder::new()
                .flexible(true)
                .from_writer(Vec::new());
            let mut count = 0;
            for row in rows {
                let row = row.as_array().context("Expected CSV row array")?;
                count += row.len();
                ensure!(count <= ITEMS, "CSV cell limit");
                let cells = row
                    .iter()
                    .map(|value| match value {
                        Value::String(s) => Ok(s.clone()),
                        Value::Null => Ok(String::new()),
                        Value::Bool(_) | Value::Number(_) => Ok(value.to_string()),
                        _ => bail!("CSV cells must be scalar"),
                    })
                    .collect::<Result<Vec<_>>>()?;
                writer.write_record(cells)?;
            }
            writer.into_inner()?
        }
        "docx" => write_docx(text(&input["text"])?)?,
        "xlsx" => write_xlsx(input)?,
        "ipynb" => {
            validate_notebook(input)?;
            serde_json::to_vec_pretty(input)?
        }
        "zip" => write_zip(input)?,
        _ => bail!("Unknown encode format {format}"),
    };
    ensure!(bytes.len() <= EXPANDED, "Encoded output exceeds limit");
    Ok(bytes)
}

fn safe_entry(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= 1024 && !name.contains(['\\', ':', '\0']),
        "Unsafe archive path"
    );
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    ensure!(
        !trimmed.is_empty() && !trimmed.starts_with('/'),
        "Absolute archive path"
    );
    let mut depth = 0;
    for part in trimmed.split('/') {
        depth += 1;
        ensure!(
            depth <= 32
                && !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' ']),
            "Unsafe archive component"
        );
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "Reserved archive component"
        );
    }
    Ok(())
}
fn archive(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(archive.len() <= ITEMS, "ZIP entry limit");
    let mut total = 0u64;
    let mut actual = 0usize;
    let mut names = BTreeSet::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        safe_entry(file.name())?;
        ensure!(
            names.insert(file.name().trim_end_matches('/').to_lowercase()),
            "Duplicate or aliased ZIP entry"
        );
        ensure!(
            file.unix_mode()
                .is_none_or(|mode| mode & 0o170000 != 0o120000),
            "ZIP links are forbidden"
        );
        total = total
            .checked_add(file.size())
            .context("ZIP size overflow")?;
        ensure!(total <= EXPANDED as u64, "Expanded ZIP limit");
        // Verify the actual stream before downstream parsers allocate from ZIP
        // metadata. A forged uncompressed-size field must not relax the bound.
        let mut buffer = [0u8; 16 * 1024];
        let mut produced = 0usize;
        loop {
            let remaining = EXPANDED.saturating_sub(actual);
            let capacity = buffer.len().min(remaining.saturating_add(1));
            let read = file.read(&mut buffer[..capacity])?;
            if read == 0 {
                break;
            }
            actual += read;
            produced += read;
            ensure!(actual <= EXPANDED, "Actual ZIP expansion exceeds limit");
        }
        ensure!(
            produced as u64 == file.size(),
            "ZIP entry size differs from content"
        );
    }
    Ok(archive)
}
fn read_zip(bytes: &[u8], contents: bool) -> Result<Value> {
    let mut zip = archive(bytes)?;
    let mut entries = Vec::new();
    let mut total = 0;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index)?;
        let mut entry = json!({"path":file.name(),"directory":file.is_dir(),"size":file.size()});
        if contents && !file.is_dir() {
            let mut data = Vec::new();
            file.by_ref()
                .take((EXPANDED - total) as u64 + 1)
                .read_to_end(&mut data)?;
            total += data.len();
            ensure!(total <= EXPANDED, "Expanded ZIP data limit");
            entry["base64"] = json!(STANDARD.encode(data));
        }
        entries.push(entry);
    }
    Ok(json!({"entries":entries}))
}
fn zip_member(bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut zip = archive(bytes)?;
    let file = zip.by_name(name)?;
    ensure!(
        file.size() <= EXPANDED as u64,
        "Document member exceeds limit"
    );
    let mut output = Vec::new();
    file.take(EXPANDED as u64 + 1).read_to_end(&mut output)?;
    ensure!(output.len() <= EXPANDED, "Expanded document limit");
    Ok(output)
}
fn write_zip(input: &Value) -> Result<Vec<u8>> {
    let entries = input["entries"]
        .as_array()
        .context("Expected archive entries")?;
    ensure!(entries.len() <= ITEMS, "ZIP entry limit");
    let mut names = BTreeSet::new();
    let mut total = 0;
    let mut prepared = Vec::new();
    for entry in entries {
        let name = text(&entry["path"])?;
        safe_entry(name)?;
        ensure!(!name.ends_with('/'), "Create accepts regular file entries");
        ensure!(
            names.insert(name.to_lowercase()),
            "Duplicate archive destination"
        );
        let data = decoded(&entry["base64"])?;
        total += data.len();
        ensure!(total <= EXPANDED, "Archive input limit");
        prepared.push((name, data));
    }
    prepared.sort_by(|a, b| a.0.cmp(b.0));
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in prepared {
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o600),
        )?;
        zip.write_all(&data)?;
    }
    Ok(zip.finish()?.into_inner())
}

fn read_docx(bytes: &[u8]) -> Result<Value> {
    let xml = zip_member(bytes, "word/document.xml")?;
    let xml = std::str::from_utf8(&xml)?;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut output = String::new();
    let mut in_text = false;
    let mut paragraphs = 0;
    loop {
        match reader.read_event()? {
            quick_xml::events::Event::Start(e) if e.local_name().as_ref() == b"t" => in_text = true,
            quick_xml::events::Event::End(e) if e.local_name().as_ref() == b"t" => in_text = false,
            quick_xml::events::Event::End(e) if e.local_name().as_ref() == b"p" => {
                output.push('\n');
                paragraphs += 1;
                ensure!(paragraphs <= ITEMS, "DOCX paragraph limit");
            }
            quick_xml::events::Event::Empty(e) if e.local_name().as_ref() == b"tab" => {
                output.push('\t')
            }
            quick_xml::events::Event::Empty(e) if e.local_name().as_ref() == b"br" => {
                output.push('\n')
            }
            quick_xml::events::Event::Text(e) if in_text => {
                output.push_str(&quick_xml::escape::unescape(&e.decode()?)?)
            }
            quick_xml::events::Event::GeneralRef(e) if in_text => {
                output.push_str(&quick_xml::escape::unescape(&format!("&{};", e.decode()?))?)
            }
            quick_xml::events::Event::DocType(_) => {
                bail!("Document type declarations are not accepted")
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
        ensure!(output.len() <= EXPANDED, "DOCX output limit");
    }
    Ok(json!({"text":output,"paragraphs":paragraphs}))
}
fn write_docx(text: &str) -> Result<Vec<u8>> {
    ensure!(text.len() <= INPUT, "Document text limit");
    ensure!(text.chars().all(|c|matches!(c,'\t'|'\n'|'\r'|'\u{20}'..='\u{d7ff}'|'\u{e000}'..='\u{fffd}'|'\u{10000}'..='\u{10ffff}')), "Text contains a character forbidden by XML 1.0");
    let paragraphs = text
        .split('\n')
        .map(|line| {
            format!(
                "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                quick_xml::escape::escape(line)
            )
        })
        .collect::<Vec<_>>();
    ensure!(paragraphs.len() <= ITEMS, "Document paragraph limit");
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{}<w:sectPr/></w:body></w:document>"#,
        paragraphs.join("")
    );
    ensure!(
        document.len().saturating_add(2048) <= EXPANDED,
        "Expanded DOCX exceeds limit"
    );
    let entries = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        ("word/document.xml", document.as_str()),
    ];
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in entries {
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )?;
        zip.write_all(content.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}
fn read_xlsx(bytes: &[u8]) -> Result<Value> {
    use calamine::{Data, Reader};
    archive(bytes)?;
    let mut workbook = calamine::Xlsx::new(Cursor::new(bytes))?;
    let names = workbook.sheet_names().to_vec();
    ensure!(names.len() <= ITEMS, "Worksheet limit");
    let mut output = Vec::new();
    let mut count = 0;
    for name in names {
        let values = workbook.worksheet_range(&name)?;
        let formulas = workbook.worksheet_formula(&name)?;
        let rows = values.get_size().0.max(formulas.get_size().0);
        let columns = values.get_size().1.max(formulas.get_size().1);
        ensure!(
            rows.saturating_mul(columns) <= 65536,
            "Worksheet grid limit"
        );
        let start = values.start().unwrap_or((0, 0));
        let mut cells = Vec::new();
        for (row, column, value) in values.cells() {
            let row = row as u32 + start.0;
            let column = column as u32 + start.1;
            let formula = formulas.get_value((row, column)).filter(|s| !s.is_empty());
            let value = match value {
                Data::Empty => Value::Null,
                Data::Bool(v) => json!(v),
                Data::Int(v) => json!(v),
                Data::Float(v) => json!(v),
                _ => json!(value.to_string()),
            };
            if value.is_null() && formula.is_none() {
                continue;
            }
            count += 1;
            ensure!(count <= ITEMS, "Cell limit");
            cells.push(json!({"row":row,"column":column,"value":value,"formula":formula}));
        }
        output.push(json!({"name":name,"cells":cells}));
    }
    Ok(json!({"sheets":output}))
}
fn write_xlsx(input: &Value) -> Result<Vec<u8>> {
    let sheets = input["sheets"].as_array().context("Expected worksheets")?;
    ensure!(
        !sheets.is_empty() && sheets.len() <= ITEMS,
        "Worksheet count limit"
    );
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let mut count = 0;
    for sheet in sheets {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name(text(&sheet["name"])?)?;
        let cells = sheet["cells"]
            .as_array()
            .context("Expected worksheet cells")?;
        for cell in cells {
            count += 1;
            ensure!(count <= ITEMS, "Cell limit");
            let row = cell["row"]
                .as_u64()
                .filter(|n| *n < 1_048_576)
                .context("Invalid XLSX row")? as u32;
            let column = cell["column"]
                .as_u64()
                .filter(|n| *n < 16384)
                .context("Invalid XLSX column")? as u16;
            ensure!(
                (row as u64 + 1) * (column as u64 + 1) <= 65536,
                "Worksheet grid limit"
            );
            if let Some(formula) = cell.get("formula") {
                worksheet.write_formula(row, column, text(formula)?)?;
            } else {
                match &cell["value"] {
                    Value::Null => {}
                    Value::Bool(v) => {
                        worksheet.write_boolean(row, column, *v)?;
                    }
                    Value::Number(v) => {
                        worksheet.write_number(
                            row,
                            column,
                            v.as_f64().context("Invalid numeric cell")?,
                        )?;
                    }
                    Value::String(v) => {
                        worksheet.write_string(row, column, v)?;
                    }
                    _ => bail!("Expected scalar cell value"),
                }
            }
        }
    }
    Ok(workbook.save_to_buffer()?)
}
fn validate_notebook(value: &Value) -> Result<()> {
    ensure!(value["nbformat"] == 4, "Only notebook v4 is supported");
    let cells = value["cells"]
        .as_array()
        .context("Missing notebook cells")?;
    ensure!(cells.len() <= ITEMS, "Notebook cell limit");
    for cell in cells {
        ensure!(
            matches!(
                cell["cell_type"].as_str(),
                Some("markdown" | "code" | "raw")
            ),
            "Unknown notebook cell type"
        );
        ensure!(
            cell["source"].is_string()
                || cell["source"]
                    .as_array()
                    .is_some_and(|items| items.iter().all(Value::is_string)),
            "Invalid notebook source"
        );
    }
    Ok(())
}

fn read_pdf(bytes: &[u8]) -> Result<Value> {
    let options = lopdf::LoadOptions {
        max_decompressed_size: Some(EXPANDED),
        ..Default::default()
    };
    let mut document = lopdf::Document::load_mem_with_options(bytes, options)?;
    ensure!(document.objects.len() <= 16384, "PDF object limit");
    let mut total = 0usize;
    for object in document.objects.values_mut() {
        if let lopdf::Object::Stream(stream) = object {
            if stream
                .dict
                .get(b"Subtype")
                .ok()
                .and_then(|v| v.as_name().ok())
                == Some(b"Image".as_slice())
            {
                continue;
            }
            let decoded = stream.get_plain_content_with_limit(EXPANDED.saturating_sub(total))?;
            total += decoded.len();
            ensure!(total <= EXPANDED, "PDF expanded stream limit");
            stream.set_plain_content(decoded);
        }
    }
    let pages = document.get_pages();
    ensure!(pages.len() <= 1024, "PDF page limit");
    let mut output = String::new();
    for number in pages.keys() {
        let text =
            document.extract_text_with_limit(&[*number], EXPANDED.saturating_sub(output.len()))?;
        ensure!(output.len() + text.len() <= EXPANDED, "PDF text limit");
        output.push_str(&text);
    }
    Ok(json!({"text":output,"pages":pages.len()}))
}
