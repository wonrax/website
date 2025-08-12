use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct Godbolt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileArgs {
    /// Compiler Explorer compiler id (e.g., gcc-13.2, clang-18, rust-1.80.0)
    pub compiler_id: String,
    /// Primary source code
    pub source: String,
    /// Compiler flags string
    #[serde(default)]
    pub user_arguments: Option<String>,
    /// Additional files: [{ filename, contents }]
    #[serde(default)]
    pub files: Option<Vec<ExtraFile>>,
    /// Libraries: [{ id, version }]
    #[serde(default)]
    pub libraries: Option<Vec<LibrarySpec>>,
    /// Optional CE language id (e.g., c++, rust)
    #[serde(default)]
    pub lang: Option<String>,
    /// Enable execution of the compiled program
    #[serde(default)]
    pub execute: Option<bool>,
    /// Optional filters map
    #[serde(default)]
    pub filters: Option<serde_json::Value>,
    /// Optional tools list
    #[serde(default)]
    pub tools: Option<serde_json::Value>,
    /// Optional compilerOptions overrides
    #[serde(default)]
    pub compiler_options: Option<serde_json::Value>,
    /// Optional executeParameters
    #[serde(default)]
    pub execute_parameters: Option<serde_json::Value>,
    /// Optional allowStoreCodeDebug
    #[serde(default)]
    pub allow_store_code_debug: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileOutput {
    pub ok: bool,
    pub code: i32,
    pub stdout: serde_json::Value,
    pub stderr: serde_json::Value,
    pub asm: serde_json::Value,
    pub compiler: String,
    pub flags: String,
    pub libraries: Vec<LibrarySpec>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraFile {
    pub filename: String,
    pub contents: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySpec {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatArgs {
    pub formatter: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatOutput {
    pub formatted: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangsOutput(pub serde_json::Value);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilersArgs {
    pub language_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrariesArgs {
    pub language_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionArgs {
    pub instruction_set: String,
    pub opcode: String,
}

#[derive(Debug, Error)]
#[error("Godbolt error: {0}")]
pub struct GodboltError(String);

const BASE_URL: &str = "https://godbolt.org";

impl Godbolt {
    fn client() -> reqwest::Client {
        use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/plain;q=0.8, */*;q=0.5"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("hhaidev-bot/1.0 (+https://hhaidev)"),
        );
        reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("client")
    }
}

impl Tool for Godbolt {
    const NAME: &'static str = "godbolt_compile";
    type Error = GodboltError;
    type Args = CompileArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "godbolt_compile".to_string(),
            description: "Compile code to assembly via Compiler Explorer (Godbolt). Returns asm and diagnostics. Use compilers/libraries discovery helpers to choose ids.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "compiler_id": {"type": "string", "description": "Compiler id from /api/compilers/{language}"},
                    "source": {"type": "string", "description": "Primary source code"},
                    "user_arguments": {"type": "string", "description": "Compiler flags", "nullable": true},
                    "files": {"type": "array", "items": {"type": "object", "properties": {"filename": {"type": "string"}, "contents": {"type": "string"}}, "required": ["filename", "contents"]}},
                    "libraries": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string"}, "version": {"type": "string"}}, "required": ["id", "version"]}},
                    "lang": {"type": "string", "nullable": true},
                    "execute": {"type": "boolean", "nullable": true, "description": "Run the program and capture output"},
                    "filters": {"type": "object", "nullable": true},
                    "tools": {"type": "array", "items": {"type": "object"}, "nullable": true},
                    "compiler_options": {"type": "object", "nullable": true},
                    "execute_parameters": {"type": "object", "nullable": true},
                    "allow_store_code_debug": {"type": "boolean", "nullable": true}
                },
                "required": ["compiler_id", "source"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = Self::client();
        let execute = args.execute.unwrap_or(false);
        let default_filters = json!({
            "binary": false,
            "binaryObject": false,
            "commentOnly": true,
            "demangle": true,
            "directives": true,
            "execute": execute,
            "intel": true,
            "labels": true,
            "libraryCode": false,
            "trim": false,
            "debugCalls": false
        });
        let payload = json!({
            "source": args.source,
            "options": {
                "userArguments": args.user_arguments.clone().unwrap_or_default(),
                "compilerOptions": args.compiler_options.clone().unwrap_or_else(|| json!({"skipAsm": false, "executorRequest": execute, "overrides": []})),
                "filters": args.filters.clone().unwrap_or(default_filters),
                "tools": args.tools.clone().unwrap_or_else(|| json!([])),
                "libraries": args.libraries.clone().unwrap_or_default(),
                "executeParameters": args.execute_parameters.clone().unwrap_or_else(|| json!({"args": [], "stdin": "", "runtimeTools": []})),
            },
            "lang": args.lang,
            "allowStoreCodeDebug": args.allow_store_code_debug.unwrap_or(false),
            "files": args.files.clone().unwrap_or_default()
        });

        let url = format!("{BASE_URL}/api/compiler/{}/compile", args.compiler_id);
        let res = client
            .post(url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| GodboltError(e.to_string()))?;
        let ct = res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let compiler = args.compiler_id.clone();
        let flags = args.user_arguments.unwrap_or_default();
        let libs = args.libraries.unwrap_or_default();

        if ct.starts_with("application/json") {
            let bytes = res.bytes().await.map_err(|e| GodboltError(e.to_string()))?;
            let data: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
                GodboltError(format!(
                    "failed to parse CE json: {e}: {}",
                    String::from_utf8_lossy(&bytes)
                ))
            })?;

            // Build part
            let build = data
                .get("buildResult")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let build_stdout = build.get("stdout").cloned().unwrap_or_else(|| json!([]));
            let build_stderr = build.get("stderr").cloned().unwrap_or_else(|| json!([]));
            // Exec part
            let did_execute = data
                .get("didExecute")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let exec_stdout = data.get("stdout").cloned().unwrap_or_else(|| json!([]));
            let exec_stderr = data.get("stderr").cloned().unwrap_or_else(|| json!([]));

            // Helper to join
            fn join_text(arr: &serde_json::Value) -> String {
                match arr {
                    serde_json::Value::Array(vs) => vs
                        .iter()
                        .filter_map(|v| {
                            if let Some(s) = v.as_str() {
                                Some(s.to_string())
                            } else if let Some(obj) = v.as_object() {
                                obj.get("text")
                                    .and_then(|t| t.as_str())
                                    .map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => String::new(),
                }
            }

            let structured = json!({
                "build": {
                    "code": build.get("code").and_then(|v| v.as_i64()).unwrap_or(-1),
                    "execTimeMs": build.get("execTime").and_then(|v| v.as_i64()),
                    "stdout": build_stdout,
                    "stderr": build_stderr,
                    "stdoutText": join_text(&build.get("stdout").cloned().unwrap_or_else(|| json!([]))),
                    "stderrText": join_text(&build.get("stderr").cloned().unwrap_or_else(|| json!([]))),
                    "timedOut": build.get("timedOut").and_then(|v| v.as_bool()).unwrap_or(false),
                    "truncated": build.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false),
                    "options": {
                        "compilationOptions": build.get("compilationOptions").cloned().unwrap_or_else(|| json!([])),
                        "inputFilename": build.get("inputFilename").cloned(),
                        "executableFilename": build.get("executableFilename").cloned(),
                        "instructionSet": build.get("instructionSet").cloned(),
                    }
                },
                "exec": if did_execute { Some(json!({
                    "code": data.get("code").and_then(|v| v.as_i64()).unwrap_or(-1),
                    "execTimeMs": data.get("execTime").and_then(|v| v.as_i64()),
                    "stdout": exec_stdout,
                    "stderr": exec_stderr,
                    "stdoutText": join_text(&data.get("stdout").cloned().unwrap_or_else(|| json!([]))),
                    "stderrText": join_text(&data.get("stderr").cloned().unwrap_or_else(|| json!([]))),
                    "timedOut": data.get("timedOut").and_then(|v| v.as_bool()).unwrap_or(false),
                    "truncated": data.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false),
                    "didExecute": true
                })) } else { None },
                "meta": {
                    "compiler": compiler,
                    "flags": flags,
                    "libraries": libs,
                    "okToCache": data.get("okToCache").and_then(|v| v.as_bool()).unwrap_or(true)
                }
            });

            Ok(structured)
        } else {
            let text = res.text().await.unwrap_or_default();
            Ok(json!({
                "build": {
                    "code": -1,
                    "execTimeMs": null,
                    "stdout": [],
                    "stderr": [{"text": text}],
                    "stdoutText": "",
                    "stderrText": text,
                    "timedOut": false,
                    "truncated": false,
                    "options": {
                        "compilationOptions": [],
                        "inputFilename": null,
                        "executableFilename": null,
                        "instructionSet": null
                    }
                },
                "exec": null,
                "meta": { "compiler": compiler, "flags": flags, "libraries": libs, "okToCache": false }
            }))
        }
    }
}

#[derive(Debug, Clone)]
pub struct GodboltFormats;

impl Tool for GodboltFormats {
    const NAME: &'static str = "godbolt_formatters";
    type Error = GodboltError;
    type Args = ();
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "godbolt_formatters".to_string(),
            description: "List available code formatters from Compiler Explorer.".to_string(),
            parameters: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = Godbolt::client();
        let url = format!("{BASE_URL}/api/formats");
        let res = client
            .get(url)
            .send()
            .await
            .map_err(|e| GodboltError(e.to_string()))?;
        res.json().await.map_err(|e| GodboltError(e.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct GodboltFormat;

impl Tool for GodboltFormat {
    const NAME: &'static str = "godbolt_format";
    type Error = GodboltError;
    type Args = FormatArgs;
    type Output = FormatOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "godbolt_format".to_string(),
            description: "Format source code using a specified formatter on Compiler Explorer."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "formatter": {"type": "string"},
                    "source": {"type": "string"}
                },
                "required": ["formatter", "source"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = Godbolt::client();
        let url = format!("{BASE_URL}/api/format/{}", args.formatter);
        let res = client
            .post(url)
            .json(&json!({"source": args.source}))
            .send()
            .await
            .map_err(|e| GodboltError(e.to_string()))?;
        let val: serde_json::Value = res.json().await.map_err(|e| GodboltError(e.to_string()))?;
        Ok(FormatOutput {
            formatted: val
                .get("answer")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct GodboltLanguages;
impl Tool for GodboltLanguages {
    const NAME: &'static str = "godbolt_languages";
    type Error = GodboltError;
    type Args = ();
    type Output = LangsOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "godbolt_languages".to_string(),
            description: "List supported languages on Compiler Explorer.".to_string(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = Godbolt::client();
        let url = format!("{BASE_URL}/api/languages");
        let res = client
            .get(url)
            .send()
            .await
            .map_err(|e| GodboltError(e.to_string()))?;
        Ok(LangsOutput(
            res.json().await.map_err(|e| GodboltError(e.to_string()))?,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct GodboltCompilers;
impl Tool for GodboltCompilers {
    const NAME: &'static str = "godbolt_compilers";
    type Error = GodboltError;
    type Args = CompilersArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "godbolt_compilers".to_string(),
            description: "List compilers for a given language id (e.g., c++, rust).".to_string(),
            parameters: json!({"type": "object", "properties": {"language_id": {"type": "string"}}, "required": ["language_id"]}),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = Godbolt::client();
        let url = format!("{BASE_URL}/api/compilers/{}", args.language_id);
        let res = client
            .get(url)
            .send()
            .await
            .map_err(|e| GodboltError(e.to_string()))?;
        res.json().await.map_err(|e| GodboltError(e.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct GodboltLibraries;
impl Tool for GodboltLibraries {
    const NAME: &'static str = "godbolt_libraries";
    type Error = GodboltError;
    type Args = LibrariesArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "godbolt_libraries".to_string(),
            description: "List libraries for a given language id.".to_string(),
            parameters: json!({"type": "object", "properties": {"language_id": {"type": "string"}}, "required": ["language_id"]}),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = Godbolt::client();
        let url = format!("{BASE_URL}/api/libraries/{}", args.language_id);
        let res = client
            .get(url)
            .send()
            .await
            .map_err(|e| GodboltError(e.to_string()))?;
        res.json().await.map_err(|e| GodboltError(e.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct GodboltAsmDoc;
impl Tool for GodboltAsmDoc {
    const NAME: &'static str = "godbolt_asm_doc";
    type Error = GodboltError;
    type Args = InstructionArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "godbolt_asm_doc".to_string(),
            description: "Get documentation for a specific assembly instruction (x86/arm/etc)."
                .to_string(),
            parameters: json!({"type": "object", "properties": {"instruction_set": {"type": "string"}, "opcode": {"type": "string"}}, "required": ["instruction_set", "opcode"]}),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = Godbolt::client();
        let url = format!(
            "{BASE_URL}/api/asm/{}/{}",
            args.instruction_set, args.opcode
        );
        let res = client
            .get(url)
            .send()
            .await
            .map_err(|e| GodboltError(e.to_string()))?;
        res.text().await.map_err(|e| GodboltError(e.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct GodboltVersion;
impl Tool for GodboltVersion {
    const NAME: &'static str = "godbolt_version";
    type Error = GodboltError;
    type Args = ();
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "godbolt_version".to_string(),
            description: "Get Compiler Explorer instance version.".to_string(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = Godbolt::client();
        let url = format!("{BASE_URL}/api/version");
        let res = client
            .get(url)
            .send()
            .await
            .map_err(|e| GodboltError(e.to_string()))?;
        res.text().await.map_err(|e| GodboltError(e.to_string()))
    }
}
