//! Hostile parser inputs run out of process: abort must fail, not kill the suite.
use codestyles::server_interface::{
    ErrorCode, MAX_CONFIG_DEPTH, Operation, PROTOCOL_VERSION, Request, Response, decode_request,
    handle,
};
use std::process::Command;
fn validate(config_jsonc: String) -> Result<Response, ErrorCode> {
    handle(Request {
        protocol_version: PROTOCOL_VERSION,
        operation: Operation::ValidateConfigSyntax { config_jsonc },
    })
}
#[test]
fn hostile_inputs_do_not_abort() {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "hostile_input_child", "--nocapture"])
        .env("CODESTYLES_DEPTH_TEST_CHILD", "1")
        .status()
        .unwrap();
    assert!(status.success(), "parser child failed: {status}");
}
#[test]
fn hostile_input_child() {
    if std::env::var_os("CODESTYLES_DEPTH_TEST_CHILD").is_none() {
        return;
    }
    for depth in [MAX_CONFIG_DEPTH, 10_000] {
        for (open, close) in [("[", "]"), ("{\"x\":", "}")] {
            assert_eq!(
                validate(format!(
                    "{{\"x\":{}0{}}}",
                    open.repeat(depth),
                    close.repeat(depth)
                )),
                Err(ErrorCode::ConfigTooDeep)
            );
        }
    }
    let bare_cr_bypass = format!(
        "{{//\r}}\"\n\"x\":{}0{}}}//\"",
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    assert_eq!(validate(bare_cr_bypass), Err(ErrorCode::ConfigTooDeep));
    let json = format!(
        "{{\"protocol_version\":1,\"operation\":{{\"type\":\"validate_config_syntax\",\"config_jsonc\":{}0{}}}}}",
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    assert_eq!(
        decode_request(json.as_bytes()),
        Err(ErrorCode::InvalidRequest)
    );
}
#[test]
fn exact_depth_and_comment_string_boundaries() {
    assert_eq!(
        validate(format!(
            "{{\"x\":{}0{}}}",
            "[".repeat(MAX_CONFIG_DEPTH - 1),
            "]".repeat(MAX_CONFIG_DEPTH - 1)
        )),
        Ok(Response::ConfigSyntax { valid: true })
    );
    let braces = "[{]}".repeat(1000);
    for config in [
        format!("{{\"x\":\"{braces}\"}}"),
        format!("{{/* {braces} */\"x\":0}}"),
        format!("{{// {braces}\r\n\"x\":0}}"),
        serde_json::json!({"x": format!("escaped quote \" {braces} trailing slash \\")})
            .to_string(),
        "{}// tail".into(),
        format!("{{}}// bare CR\r{braces}"),
    ] {
        assert_eq!(
            validate(config.clone()),
            Ok(Response::ConfigSyntax { valid: true }),
            "{config}"
        );
    }
    for config in [
        "{]",
        "[}",
        "{",
        "{\"x\":\"unterminated}",
        "{}/* unterminated",
        "{}/*/",
        "{\"x\":\"escape\\",
    ] {
        assert_eq!(
            validate(config.into()),
            Ok(Response::ConfigSyntax { valid: false })
        );
    }
}
