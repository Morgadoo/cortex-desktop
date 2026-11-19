//! Runs a tool the assistant wrote: a JavaScript script that defines `function main(args)`.
//!
//! The script reaches nothing but the three functions the host gives it, each checked by the
//! host against the tool's capabilities:
//!
//! - `log(message)`: a line for the activity log, never shown to the model;
//! - `fetch(method, url, body?)`: `{ status, body }`, body as text;
//! - `read(path)`: a workspace file as text;
//! - `tool(name, args)`: another approved tool's result, when this one may call it.
//!
//! `main` returns any value that JSON can hold; that is the tool's result. A script without `main`
//! runs as a program, and what it prints with `console.log` or `print` is the result.

wit_bindgen::generate!({ path: "../wit", world: "script-tool" });

use std::cell::RefCell;

use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{js_string, Context, JsResult, JsValue, NativeFunction, Source};
use cortex::script::host;

thread_local! {
    /// What a program printed, in order.
    static PRINTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct Runtime;

fn text_arg(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<String> {
    match args.get(i) {
        Some(v) if !v.is_undefined() && !v.is_null() => {
            Ok(v.to_string(ctx)?.to_std_string_escaped())
        }
        _ => Ok(String::new()),
    }
}

fn error(message: String) -> boa_engine::JsError {
    boa_engine::JsNativeError::error()
        .with_message(message)
        .into()
}

fn log(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    host::log(&text_arg(args, 0, ctx)?);
    Ok(JsValue::undefined())
}

fn fetch(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let method = text_arg(args, 0, ctx)?;
    let url = text_arg(args, 1, ctx)?;
    let body = text_arg(args, 2, ctx)?;
    let body = (!body.is_empty()).then(|| body.into_bytes());
    let response = host::http_request(&method, &url, body.as_deref()).map_err(error)?;
    let json = serde_json::json!({
        "status": response.status,
        "body": String::from_utf8_lossy(&response.body),
    });
    JsValue::from_json(&json, ctx)
}

fn read(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let path = text_arg(args, 0, ctx)?;
    let bytes = host::workspace_read(&path).map_err(error)?;
    Ok(JsValue::from(js_string!(String::from_utf8_lossy(&bytes).as_ref())))
}

fn print(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let mut parts = Vec::new();
    for value in args {
        parts.push(if value.is_string() {
            value.to_string(ctx)?.to_std_string_escaped()
        } else {
            value
                .to_json(ctx)?
                .map_or_else(|| "undefined".to_string(), |json| json.to_string())
        });
    }
    PRINTED.with(|p| p.borrow_mut().push(parts.join(" ")));
    Ok(JsValue::undefined())
}

fn tool(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = text_arg(args, 0, ctx)?;
    let input = match args.get(1) {
        Some(v) if !v.is_undefined() => v
            .to_json(ctx)?
            .map_or_else(|| "{}".to_string(), |json| json.to_string()),
        _ => "{}".to_string(),
    };
    let output = host::call_tool(&name, &input).map_err(error)?;
    match serde_json::from_str::<serde_json::Value>(&output) {
        Ok(json) => JsValue::from_json(&json, ctx),
        Err(_) => Ok(JsValue::from(js_string!(output.as_str()))),
    }
}

impl Guest for Runtime {
    fn run(script: String, args: String) -> Result<String, String> {
        let mut ctx = Context::default();
        let register = |ctx: &mut Context, name, f| {
            ctx.register_global_callable(name, 1, NativeFunction::from_fn_ptr(f))
                .map_err(|e| e.to_string())
        };
        register(&mut ctx, js_string!("log"), log)?;
        register(&mut ctx, js_string!("fetch"), fetch)?;
        register(&mut ctx, js_string!("read"), read)?;
        register(&mut ctx, js_string!("tool"), tool)?;
        register(&mut ctx, js_string!("print"), print)?;
        let console = ObjectInitializer::new(&mut ctx)
            .function(NativeFunction::from_fn_ptr(print), js_string!("log"), 0)
            .function(NativeFunction::from_fn_ptr(print), js_string!("error"), 0)
            .build();
        ctx.register_global_property(js_string!("console"), console, Attribute::all())
            .map_err(|e| e.to_string())?;
        PRINTED.with(|p| p.borrow_mut().clear());
        if !script.contains("function main") {
            // A program: it runs top to bottom, and what it printed is the result.
            ctx.eval(Source::from_bytes(&script))
                .map_err(|e| format!("the program failed: {e}"))?;
            return Ok(PRINTED.with(|p| p.borrow().join("\n")));
        }
        ctx.eval(Source::from_bytes(&script))
            .map_err(|e| format!("the script failed to load: {e}"))?;
        // The arguments arrive as a JSON string literal, so no text of theirs becomes code.
        let call = format!(
            "JSON.stringify(main(JSON.parse({})))",
            serde_json::to_string(&args).map_err(|e| e.to_string())?
        );
        let value = ctx
            .eval(Source::from_bytes(&call))
            .map_err(|e| format!("main failed: {e}"))?;
        if value.is_undefined() {
            return Ok("null".into());
        }
        Ok(value
            .to_string(&mut ctx)
            .map_err(|e| e.to_string())?
            .to_std_string_escaped())
    }
}

export!(Runtime);
