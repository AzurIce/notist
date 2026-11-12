//! Build-scoped JSON Lines events. One process handles both site boundaries.
use crate::{
    Error, Page, SiteConfig,
    extensions::{CompileInput, SiteContext, SiteExtension, TemplateInput},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

/// The event stream is incompatible with the single-request protocol.
pub const PROTOCOL_VERSION: u32 = 2;

/// A contributed file is either UTF-8 text or standard padded base64 bytes.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileContent {
    Text(String),
    Base64(String),
}

impl FileContent {
    fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self::Text(text.to_owned()),
            Err(_) => Self::Base64(STANDARD.encode(bytes)),
        }
    }

    fn into_bytes(self, path: &Path) -> Result<Vec<u8>, Error> {
        match self {
            Self::Text(text) => Ok(text.into_bytes()),
            Self::Base64(encoded) => STANDARD.decode(encoded).map_err(|error| {
                Error::Message(format!(
                    "invalid base64 for site file `{}`: {error}",
                    path.display()
                ))
            }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub root: PathBuf,
    pub config: SiteConfig,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    BeforeCompile {
        documents: BTreeMap<PathBuf, String>,
    },
    BeforeTemplate {
        pages: Vec<Page>,
        data: BTreeMap<String, serde_json::Value>,
        #[serde(default)]
        files: BTreeMap<PathBuf, FileContent>,
    },
}

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum EventRef<'a> {
    BeforeCompile {
        documents: &'a BTreeMap<PathBuf, String>,
    },
    BeforeTemplate {
        pages: &'a [Page],
        data: &'a BTreeMap<String, serde_json::Value>,
        files: BTreeMap<&'a Path, FileContent>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    BeforeCompile {
        version: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        documents: Option<BTreeMap<PathBuf, String>>,
        #[serde(default)]
        errors: Vec<String>,
    },
    BeforeTemplate {
        version: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<BTreeMap<String, serde_json::Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        files: Option<BTreeMap<PathBuf, FileContent>>,
        #[serde(default)]
        errors: Vec<String>,
    },
}

impl Response {
    fn validate(&self, expected: &str) -> Result<(), Error> {
        let (event, version, errors) = match self {
            Self::BeforeCompile {
                version, errors, ..
            } => ("before_compile", *version, errors),
            Self::BeforeTemplate {
                version, errors, ..
            } => ("before_template", *version, errors),
        };
        if version != PROTOCOL_VERSION {
            return Err(Error::Message(format!(
                "unsupported site extension response version {version}"
            )));
        }
        if event != expected {
            return Err(Error::Message(format!("expected {expected} response")));
        }
        if !errors.is_empty() {
            return Err(Error::Message(errors.join("\n")));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct CommandExtension {
    name: String,
    command: Vec<String>,
    process: Option<Process>,
}

// Clone the configuration, never a running process or its intermediate state.
impl Clone for CommandExtension {
    fn clone(&self) -> Self {
        Self::named(self.name.clone(), self.command.clone())
    }
}

impl CommandExtension {
    pub fn new(command: Vec<String>) -> Self {
        Self::named(
            command.first().cloned().unwrap_or_else(|| "command".into()),
            command,
        )
    }

    pub fn named(name: String, command: Vec<String>) -> Self {
        Self {
            name,
            command,
            process: None,
        }
    }

    fn exchange(
        &mut self,
        context: &SiteContext<'_>,
        event: EventRef<'_>,
    ) -> Result<Response, Error> {
        #[derive(Serialize)]
        struct RequestRef<'a, E> {
            version: u32,
            root: &'a Path,
            config: &'a SiteConfig,
            #[serde(flatten)]
            event: E,
        }
        if self.process.is_none() {
            self.process = Some(Process::start(&self.command, context.resources.root())?);
        }
        let expected = match &event {
            EventRef::BeforeCompile { .. } => "before_compile",
            EventRef::BeforeTemplate { .. } => "before_template",
        };
        self.process.as_mut().unwrap().exchange(
            &RequestRef {
                version: PROTOCOL_VERSION,
                root: context.resources.root(),
                config: context.config,
                event,
            },
            expected,
        )
    }
}

impl SiteExtension for CommandExtension {
    fn name(&self) -> &str {
        &self.name
    }

    fn before_compile(
        &mut self,
        context: &SiteContext<'_>,
        input: &mut CompileInput,
    ) -> Result<(), Error> {
        let event = EventRef::BeforeCompile {
            documents: &input.documents,
        };
        match self.exchange(context, event)? {
            Response::BeforeCompile { documents, .. } => {
                if let Some(documents) = documents {
                    input.documents = documents;
                }
                Ok(())
            }
            _ => Err(Error::Message("expected before_compile response".into())),
        }
    }

    fn before_template(
        &mut self,
        context: &SiteContext<'_>,
        input: &mut TemplateInput<'_>,
    ) -> Result<(), Error> {
        let event = EventRef::BeforeTemplate {
            pages: input.pages,
            data: &input.data,
            files: input
                .files
                .iter()
                .map(|(path, bytes)| (path.as_path(), FileContent::from_bytes(bytes)))
                .collect(),
        };
        match self.exchange(context, event)? {
            Response::BeforeTemplate { data, files, .. } => {
                if let Some(data) = data {
                    input.data = data;
                }
                if let Some(files) = files {
                    input.files = files
                        .into_iter()
                        .map(|(path, content)| {
                            let bytes = content.into_bytes(&path)?;
                            Ok((path, bytes))
                        })
                        .collect::<Result<_, Error>>()?;
                }
                Ok(())
            }
            _ => Err(Error::Message("expected before_template response".into())),
        }
    }

    fn finish(&mut self) -> Result<(), Error> {
        if let Some(mut process) = self.process.take() {
            process.finish()?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Process {
    fn start(command: &[String], root: &Path) -> Result<Self, Error> {
        let program = command
            .first()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Message("command requires a nonempty program".into()))?;
        let mut child = Command::new(program)
            .args(&command[1..])
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| Error::Message(format!("cannot start command: {e}")))?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    fn exchange(&mut self, request: &impl Serialize, expected: &str) -> Result<Response, Error> {
        let mut bytes = serde_json::to_vec(request)
            .map_err(|e| Error::Message(format!("cannot encode request: {e}")))?;
        bytes.push(b'\n');
        let mut line = String::new();
        let stdin = self.stdin.as_mut().unwrap();
        let stdout = &mut self.stdout;
        // Drain responses while writing. Reject invalid/early error responses
        // before joining the writer: the peer might not consume the request.
        let (written, response) = std::thread::scope(|scope| {
            let writer = scope.spawn(|| stdin.write_all(&bytes).and_then(|()| stdin.flush()));
            let response = (|| {
                let count = stdout
                    .read_line(&mut line)
                    .map_err(|e| Error::Message(format!("cannot read response: {e}")))?;
                if count == 0 {
                    if self.child.try_wait().ok().flatten().is_none() {
                        let _ = self.child.kill();
                    }
                    let status = self
                        .child
                        .wait()
                        .map_err(|e| Error::Message(e.to_string()))?;
                    return Err(Error::Message(format!(
                        "command exited with {status} before responding"
                    )));
                }
                if !line.ends_with('\n') {
                    return Err(Error::Message("response must end with a newline".into()));
                }
                let response: Response = serde_json::from_str(&line)
                    .map_err(|e| Error::Message(format!("invalid JSON response: {e}")))?;
                response.validate(expected)?;
                Ok(response)
            })();
            if response.is_err() {
                let _ = self.child.kill();
            }
            (writer.join().unwrap(), response)
        });
        let response = response?;
        written.map_err(|e| Error::Message(format!("cannot write request: {e}")))?;
        Ok(response)
    }

    fn finish(&mut self) -> Result<(), Error> {
        drop(self.stdin.take());
        let mut extra = String::new();
        self.stdout
            .read_to_string(&mut extra)
            .map_err(|e| Error::Message(format!("cannot read command output: {e}")))?;
        let status = self
            .child
            .wait()
            .map_err(|e| Error::Message(format!("cannot wait for command: {e}")))?;
        if !status.success() {
            return Err(Error::Message(format!("command exited with {status}")));
        }
        if !extra.trim().is_empty() {
            return Err(Error::Message(
                "unexpected output after final response".into(),
            ));
        }
        Ok(())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // Also runs on invalid responses and failures in later compiler stages.
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
