//! Document extraction engines (Open WebUI's content extraction): the built-in readers (PDF,
//! Word, PowerPoint, OpenDocument, HTML, text), or a server the user runs, for files the built-in
//! readers handle poorly (scans, complex layouts, tables) or not at all:
//!
//! - **Apache Tika** (`docker run -p 9998:9998 apache/tika`): `PUT /tika` with the file returns
//!   its text, with OCR when the server has Tesseract.
//! - **Docling** (`docling-serve`, port 5001): `POST /v1/convert/file` returns Markdown that keeps
//!   headings and tables.
//!
//! Used for documents added to libraries and files attached in chat; when the server fails, the
//! built-in reader takes over.

use crate::support::settings_file::SettingsFile;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Extraction {
    /// `builtin`, `tika` or `docling`.
    pub engine: String,
    pub url: String,
}

impl Extraction {
    pub fn external(&self) -> bool {
        matches!(self.engine.as_str(), "tika" | "docling") && !self.url.trim().is_empty()
    }
}

const EXTRACTION_FILE: SettingsFile<Extraction> = SettingsFile::sealed("chat/extraction.json");

pub fn load(config_dir: &Path) -> Extraction {
    EXTRACTION_FILE.load(config_dir)
}

pub fn save(config_dir: &Path, e: &Extraction) -> Result<(), String> {
    if !matches!(e.engine.as_str(), "builtin" | "tika" | "docling" | "") {
        return Err("Unknown extraction engine.".into());
    }
    if e.engine != "builtin"
        && !e.engine.is_empty()
        && !(e.url.starts_with("http://") || e.url.starts_with("https://"))
    {
        return Err("Give the server's address, like http://localhost:9998.".into());
    }
    EXTRACTION_FILE.save(config_dir, e).map_err(String::from)
}

/// Text from Docling's answer: Markdown, else plain text.
pub fn docling_text(body: &serde_json::Value) -> Option<String> {
    let doc = body
        .get("document")
        .or_else(|| body.get("documents").and_then(|d| d.get(0)))?;
    doc.get("md_content")
        .or_else(|| doc.get("text_content"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|t| !t.trim().is_empty())
}

/// Converts a file with the configured server.
pub async fn convert(e: &Extraction, name: &str, bytes: Vec<u8>) -> Result<String, String> {
    let base = e.url.trim().trim_end_matches('/');
    let client = crate::support::http::client(std::time::Duration::from_secs(300))?;
    match e.engine.as_str() {
        "tika" => {
            let r = client
                .put(format!("{base}/tika"))
                .header("Accept", "text/plain")
                .header(
                    "Content-Disposition",
                    format!("attachment; filename=\"{}\"", name.replace('"', "")),
                )
                .body(bytes)
                .send()
                .await
                .map_err(|err| format!("Tika did not answer at {base}: {err}"))?;
            if !r.status().is_success() {
                return Err(format!("Tika answered {}.", r.status()));
            }
            let text = r.text().await.map_err(|e| e.to_string())?;
            if text.trim().is_empty() {
                Err("Tika found no text.".into())
            } else {
                Ok(text)
            }
        }
        "docling" => {
            let part = reqwest::multipart::Part::bytes(bytes).file_name(name.to_string());
            let form = reqwest::multipart::Form::new()
                .part("files", part)
                .text("to_formats", "md");
            let r = client
                .post(format!("{base}/v1/convert/file"))
                .multipart(form)
                .send()
                .await
                .map_err(|err| format!("Docling did not answer at {base}: {err}"))?;
            if !r.status().is_success() {
                return Err(format!("Docling answered {}.", r.status()));
            }
            let body: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
            docling_text(&body).ok_or_else(|| "Docling found no text.".into())
        }
        _ => Err("No extraction server is set.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A one-page PDF saying `text`, with a correct cross-reference table.
    fn pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 18 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        out
    }

    /// With an Apache Tika server at `CORTEX_TEST_TIKA` (e.g. http://localhost:9998): a PDF and a
    /// web page come back as their text, and a server that is not there is reported plainly.
    #[tokio::test]
    #[ignore]
    async fn a_tika_server_reads_documents() {
        let Ok(url) = std::env::var("CORTEX_TEST_TIKA") else {
            return;
        };
        let tika = Extraction {
            engine: "tika".into(),
            url,
        };
        let text = convert(&tika, "invoice.pdf", pdf("Invoice 2041 is due on 30 June"))
            .await
            .unwrap();
        assert!(text.contains("Invoice 2041 is due on 30 June"), "{text}");
        let html =
            "<html><body><h1>Travel policy</h1><p>Book trains under four hours.</p></body></html>";
        let text = convert(&tika, "policy.html", html.as_bytes().to_vec())
            .await
            .unwrap();
        assert!(
            text.contains("Travel policy") && text.contains("Book trains under four hours."),
            "{text}"
        );
        let gone = Extraction {
            engine: "tika".into(),
            url: "http://127.0.0.1:9".into(),
        };
        assert!(convert(&gone, "x.pdf", pdf("x"))
            .await
            .unwrap_err()
            .starts_with("Tika did not answer"));
    }

    #[test]
    fn settings_and_docling_answers() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!load(dir.path()).external());
        assert!(save(
            dir.path(),
            &Extraction {
                engine: "tika".into(),
                url: "localhost".into()
            }
        )
        .is_err());
        save(
            dir.path(),
            &Extraction {
                engine: "docling".into(),
                url: "http://localhost:5001".into(),
            },
        )
        .unwrap();
        assert!(load(dir.path()).external());
        assert_eq!(
            docling_text(&json!({"document": {"md_content": "# Title\n| a | b |"}})).as_deref(),
            Some("# Title\n| a | b |")
        );
        assert_eq!(
            docling_text(&json!({"document": {"md_content": "", "text_content": "plain"}})),
            None
        );
        assert_eq!(
            docling_text(&json!({"documents": [{"text_content": "plain"}]})).as_deref(),
            Some("plain")
        );
    }

    #[tokio::test]
    async fn tika_and_docling_servers_are_called() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // A one-shot HTTP server answering like Tika, then like Docling.
        async fn serve(
            body: &'static str,
            ctype: &'static str,
        ) -> (String, tokio::task::JoinHandle<String>) {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 65536];
                let mut got = Vec::new();
                loop {
                    let n = s.read(&mut buf).await.unwrap();
                    got.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&got).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let len = text
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if got.len() >= head_end + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                s.write_all(reply.as_bytes()).await.unwrap();
                String::from_utf8_lossy(&got).to_string()
            });
            (url, task)
        }
        let (url, req) = serve("Invoice 7731\nTotal 120 EUR", "text/plain").await;
        let text = convert(
            &Extraction {
                engine: "tika".into(),
                url,
            },
            "inv.pdf",
            b"%PDF-fake".to_vec(),
        )
        .await
        .unwrap();
        assert_eq!(text, "Invoice 7731\nTotal 120 EUR");
        let seen = req.await.unwrap();
        assert!(seen.starts_with("PUT /tika") && seen.contains("%PDF-fake"));
        let (url, req) = serve(
            r##"{"document":{"md_content":"# Report\n\n| q | v |\n|---|---|\n| 1 | 9 |"}}"##,
            "application/json",
        )
        .await;
        let text = convert(
            &Extraction {
                engine: "docling".into(),
                url,
            },
            "r.pdf",
            b"%PDF-x".to_vec(),
        )
        .await
        .unwrap();
        assert!(text.starts_with("# Report") && text.contains("| 1 | 9 |"));
        let seen = req.await.unwrap();
        assert!(
            seen.starts_with("POST /v1/convert/file")
                && seen.contains("name=\"files\"")
                && seen.contains("to_formats")
        );
    }
}
