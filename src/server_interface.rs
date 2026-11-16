//! Public core interface shared by future self-hosted and hosted adapters.
//!
//! No listener, authentication, filesystem paths or formatter execution lives here.
//! Syntactically valid configuration is NOT trusted to execute.
use crate::config::ConfigDocument;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
/// Byte limits, not character limits. Network adapters must also bound body reads.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_CONFIG_BYTES: usize = 256 * 1024;
/// Maximum combined object/array nesting, including the root object.
pub const MAX_CONFIG_DEPTH: usize = 64;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol_version: u32,
    pub operation: Operation,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Capabilities {},
    StandaloneConfig {},
    ValidateConfigSyntax { config_jsonc: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Capabilities {
        protocol_version: u32,
        operations: Vec<String>,
        max_request_bytes: usize,
        max_config_bytes: usize,
        max_config_depth: usize,
    },
    StandaloneConfig {
        config_jsonc: String,
    },
    ConfigSyntax {
        valid: bool,
    },
}

/// Stable public errors contain no raw parser diagnostics, paths or request text.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    RequestTooLarge,
    InvalidRequest,
    UnsupportedProtocol,
    ConfigTooLarge,
    ConfigTooDeep,
    InternalError,
}

/// HTTP adapters must enforce this body limit while reading, before allocating
/// a complete request. Do not log request bodies. Keep serde's recursion limit.
pub fn decode_request(bytes: &[u8]) -> Result<Request, ErrorCode> {
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(ErrorCode::RequestTooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| ErrorCode::InvalidRequest)
}

/// Iterative lexical guard before the recursive JSONC CST parser (and drop).
/// Strings/comments cannot contribute delimiters. Mismatched closers fail before
/// parsing rather than reducing depth and hiding an open container.
/// Other JSONC syntax remains the responsibility of the existing core parser.
fn bounded_config_structure(text: &str) -> Result<bool, ErrorCode> {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        String,
        Escape,
        LineComment,
        BlockComment,
    }
    let bytes = text.as_bytes();
    let mut state = State::Code;
    let mut stack = Vec::with_capacity(MAX_CONFIG_DEPTH);
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            State::Code => match byte {
                b'"' => state = State::String,
                b'/' if next == Some(b'/') => {
                    state = State::LineComment;
                    index += 1;
                }
                b'/' if next == Some(b'*') => {
                    state = State::BlockComment;
                    index += 1;
                }
                b'{' | b'[' => {
                    if stack.len() == MAX_CONFIG_DEPTH {
                        return Err(ErrorCode::ConfigTooDeep);
                    }
                    stack.push(if byte == b'{' { b'}' } else { b']' });
                }
                b'}' | b']' => {
                    if stack.pop() != Some(byte) {
                        return Ok(false);
                    }
                }
                _ => {}
            },
            State::String => match byte {
                b'\\' => state = State::Escape,
                b'"' => state = State::Code,
                _ => {}
            },
            State::Escape => state = State::String,
            State::LineComment => {
                // jsonc_parser ends line comments on LF or CRLF, not bare CR.
                if byte == b'\n' {
                    state = State::Code;
                }
            }
            State::BlockComment => {
                if byte == b'*' && next == Some(b'/') {
                    state = State::Code;
                    index += 1;
                }
            }
        }
        index += 1;
    }
    Ok(stack.is_empty() && matches!(state, State::Code | State::LineComment))
}

/// In-process implementation available without desktop or private server code.
/// Validation checks JSONC object syntax, not plugin settings, trust or execution.
pub fn handle(request: Request) -> Result<Response, ErrorCode> {
    if request.protocol_version != PROTOCOL_VERSION {
        return Err(ErrorCode::UnsupportedProtocol);
    }
    match request.operation {
        Operation::Capabilities {} => Ok(Response::Capabilities {
            protocol_version: PROTOCOL_VERSION,
            operations: vec![
                "capabilities".into(),
                "standalone_config".into(),
                "validate_config_syntax".into(),
            ],
            max_request_bytes: MAX_REQUEST_BYTES,
            max_config_bytes: MAX_CONFIG_BYTES,
            max_config_depth: MAX_CONFIG_DEPTH,
        }),
        Operation::StandaloneConfig {} => {
            let doc = ConfigDocument::standalone_pinned().map_err(|_| ErrorCode::InternalError)?;
            Ok(Response::StandaloneConfig {
                config_jsonc: doc.draft_text().to_owned(),
            })
        }
        Operation::ValidateConfigSyntax { config_jsonc } => {
            if config_jsonc.len() > MAX_CONFIG_BYTES {
                return Err(ErrorCode::ConfigTooLarge);
            }
            if !bounded_config_structure(&config_jsonc)? {
                return Ok(Response::ConfigSyntax { valid: false });
            }
            let mut doc =
                ConfigDocument::standalone_pinned().map_err(|_| ErrorCode::InternalError)?;
            Ok(Response::ConfigSyntax {
                valid: doc.replace_draft_text(&config_jsonc).is_ok(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(operation: Operation) -> Request {
        Request {
            protocol_version: PROTOCOL_VERSION,
            operation,
        }
    }

    #[test]
    fn wire_roundtrip_and_capabilities() {
        let req = request(Operation::Capabilities {});
        let decoded = decode_request(&serde_json::to_vec(&req).unwrap()).unwrap();
        assert_eq!(decoded, req);
        let result = handle(decoded).unwrap();
        assert_eq!(
            serde_json::from_value::<Response>(serde_json::to_value(&result).unwrap()).unwrap(),
            result
        );
        if let Response::Capabilities { operations, .. } = result {
            assert_eq!(operations.len(), 3);
            assert!(!operations.iter().any(|op| op == "format"));
        } else {
            panic!("unexpected response");
        }
    }
    #[test]
    fn rejects_unknown_fields_operations_and_protocols() {
        for json in [
            r#"{"protocol_version":1,"operation":{"type":"capabilities"},"path":"/etc/passwd"}"#,
            r#"{"protocol_version":1,"operation":{"type":"capabilities","path":"x"}}"#,
            r#"{"protocol_version":1,"operation":{"type":"format"}}"#,
            r#"{"protocol_version":1,"protocol_version":2,"operation":{"type":"capabilities"}}"#,
        ] {
            assert_eq!(
                decode_request(json.as_bytes()),
                Err(ErrorCode::InvalidRequest)
            );
        }
        assert_eq!(
            handle(Request {
                protocol_version: 2,
                operation: Operation::Capabilities {}
            }),
            Err(ErrorCode::UnsupportedProtocol)
        );
    }
    #[test]
    fn bounds_wire_and_decoded_utf8_bytes() {
        assert_eq!(
            decode_request(&vec![b' '; MAX_REQUEST_BYTES + 1]),
            Err(ErrorCode::RequestTooLarge)
        );
        assert_eq!(
            handle(request(Operation::ValidateConfigSyntax {
                config_jsonc: "é".repeat(MAX_CONFIG_BYTES / 2 + 1)
            })),
            Err(ErrorCode::ConfigTooLarge)
        );
    }
    #[test]
    fn syntax_matches_native_editor_not_execution_trust() {
        for (text, valid) in [
            ("{/* comments */ \"unknown\":true,}", true),
            ("{\"extends\":\"https://example.invalid/config\"}", true),
            ("[]", false),
            ("{", false),
            ("null", false),
        ] {
            assert_eq!(
                handle(request(Operation::ValidateConfigSyntax {
                    config_jsonc: text.into()
                })),
                Ok(Response::ConfigSyntax { valid })
            );
        }
    }
    #[test]
    fn standalone_matches_existing_core_exactly() {
        assert_eq!(
            handle(request(Operation::StandaloneConfig {})),
            Ok(Response::StandaloneConfig {
                config_jsonc: ConfigDocument::standalone_pinned()
                    .unwrap()
                    .draft_text()
                    .to_owned()
            })
        );
    }
}
