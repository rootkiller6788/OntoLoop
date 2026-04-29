use anyhow::Result;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreprocessFormat {
    Html,
    Json,
    Csv,
    Docx,
    Pdf,
    Markdown,
    PlainText,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MarkdownPreprocessRequest {
    pub source_path: Option<String>,
    pub content: String,
    pub declared_format: Option<String>,
    #[serde(default)]
    pub options: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MarkdownPreprocessResult {
    pub plugin_id: String,
    pub source_format: PreprocessFormat,
    pub target_format: String,
    pub markdown: String,
    pub content_digest: String,
    pub bytes_in: usize,
    pub bytes_out: usize,
    pub warnings: Vec<String>,
    pub metadata: serde_json::Value,
}

pub struct MarkdownPreprocessPlugin;

impl MarkdownPreprocessPlugin {
    pub const PLUGIN_ID: &'static str = "plugin:markdown-preprocess:v1";

    pub fn preprocess(request: &MarkdownPreprocessRequest) -> Result<MarkdownPreprocessResult> {
        let source_format = detect_format(
            request.declared_format.as_deref(),
            request.source_path.as_deref(),
        );
        let mut warnings = Vec::<String>::new();

        let markdown = match source_format {
            PreprocessFormat::Html => html_to_markdown(&request.content),
            PreprocessFormat::Json => json_to_markdown(&request.content, &mut warnings),
            PreprocessFormat::Csv => csv_to_markdown(&request.content),
            PreprocessFormat::Docx => binary_doc_to_markdown("docx", &request.content, &mut warnings),
            PreprocessFormat::Pdf => binary_doc_to_markdown("pdf", &request.content, &mut warnings),
            PreprocessFormat::Markdown | PreprocessFormat::PlainText => request.content.clone(),
        };

        let content_digest = crate::observability::event_stream::digest_value(&serde_json::json!({
            "plugin": Self::PLUGIN_ID,
            "source_path": request.source_path,
            "declared_format": request.declared_format,
            "source_format": source_format,
            "content": request.content,
            "markdown": markdown,
        }));

        Ok(MarkdownPreprocessResult {
            plugin_id: Self::PLUGIN_ID.to_string(),
            source_format,
            target_format: "markdown".to_string(),
            bytes_in: request.content.len(),
            bytes_out: markdown.len(),
            markdown,
            content_digest,
            warnings,
            metadata: serde_json::json!({
                "source_path": request.source_path,
                "declared_format": request.declared_format,
                "options": request.options,
            }),
        })
    }
}

fn detect_format(declared: Option<&str>, source_path: Option<&str>) -> PreprocessFormat {
    let candidate = declared
        .map(|item| item.trim().to_ascii_lowercase())
        .or_else(|| {
            source_path.and_then(|path| {
                std::path::Path::new(path)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.to_ascii_lowercase())
            })
        })
        .unwrap_or_else(|| "txt".to_string());

    match candidate.as_str() {
        "html" | "htm" => PreprocessFormat::Html,
        "json" => PreprocessFormat::Json,
        "csv" => PreprocessFormat::Csv,
        "docx" => PreprocessFormat::Docx,
        "pdf" => PreprocessFormat::Pdf,
        "md" | "markdown" => PreprocessFormat::Markdown,
        _ => PreprocessFormat::PlainText,
    }
}

fn html_to_markdown(input: &str) -> String {
    let mut out = input.replace("\r\n", "\n");
    for (from, to) in [
        ("<br>", "\n"),
        ("<br/>", "\n"),
        ("<br />", "\n"),
        ("</p>", "\n\n"),
        ("</div>", "\n"),
        ("</h1>", "\n\n"),
        ("</h2>", "\n\n"),
        ("</h3>", "\n\n"),
        ("<li>", "- "),
        ("</li>", "\n"),
    ] {
        out = out.replace(from, to);
    }
    strip_html_tags(&out)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn strip_html_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn json_to_markdown(input: &str, warnings: &mut Vec<String>) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(input) {
        let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| input.to_string());
        format!("```json\n{pretty}\n```")
    } else {
        warnings.push("json_parse_failed_fallback_plain".to_string());
        format!("```text\n{}\n```", input.trim())
    }
}

fn csv_to_markdown(input: &str) -> String {
    let rows = input
        .lines()
        .map(|line| split_csv_line(line))
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return String::new();
    }
    let header = &rows[0];
    let mut out = String::new();
    out.push('|');
    out.push_str(&header.join("|"));
    out.push_str("|\n|");
    out.push_str(&header.iter().map(|_| "---").collect::<Vec<_>>().join("|"));
    out.push('|');
    for row in rows.iter().skip(1) {
        out.push('\n');
        out.push('|');
        out.push_str(&row.join("|"));
        out.push('|');
    }
    out
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::<String>::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let chars = line.chars().peekable();
    for ch in chars {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() || line.ends_with(',') {
        fields.push(current.trim().to_string());
    }
    fields
}

fn binary_doc_to_markdown(kind: &str, input: &str, warnings: &mut Vec<String>) -> String {
    warnings.push(format!("{kind}_binary_parse_not_enabled_using_text_fallback"));
    format!(
        "# Imported {kind} document\n\n> Converted via fallback adapter.\n\n```text\n{}\n```",
        input.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::{MarkdownPreprocessPlugin, MarkdownPreprocessRequest, PreprocessFormat};

    #[test]
    fn preprocess_html_to_markdown() {
        let result = MarkdownPreprocessPlugin::preprocess(&MarkdownPreprocessRequest {
            source_path: Some("docs/page.html".to_string()),
            content: "<h1>Title</h1><p>Hello<br/>World</p>".to_string(),
            declared_format: None,
            options: serde_json::json!({}),
        })
        .expect("preprocess");
        assert_eq!(result.source_format, PreprocessFormat::Html);
        assert!(result.markdown.contains("Title"));
        assert!(result.markdown.contains("Hello"));
    }

    #[test]
    fn preprocess_json_to_markdown_fence() {
        let result = MarkdownPreprocessPlugin::preprocess(&MarkdownPreprocessRequest {
            source_path: Some("docs/raw.json".to_string()),
            content: "{\"k\":1}".to_string(),
            declared_format: None,
            options: serde_json::json!({}),
        })
        .expect("preprocess");
        assert_eq!(result.source_format, PreprocessFormat::Json);
        assert!(result.markdown.starts_with("```json"));
    }

    #[test]
    fn preprocess_csv_to_markdown_table() {
        let result = MarkdownPreprocessPlugin::preprocess(&MarkdownPreprocessRequest {
            source_path: Some("docs/raw.csv".to_string()),
            content: "name,score\nalice,10\nbob,20".to_string(),
            declared_format: None,
            options: serde_json::json!({}),
        })
        .expect("preprocess");
        assert_eq!(result.source_format, PreprocessFormat::Csv);
        assert!(result.markdown.contains("|name|score|"));
        assert!(result.markdown.contains("|alice|10|"));
    }
}
