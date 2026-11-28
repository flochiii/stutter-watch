//! One service-owned recorder; Java never passes native pointers.
use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::jstring,
};
use std::sync::{Mutex, OnceLock};
use stutter_watch::mobile::Session;

static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();

fn dispatch(command: &str, input: &str) -> Result<String, String> {
    let mut slot = SESSION
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "Recorder lock poisoned".to_string())?;
    if command == "open" {
        if slot.is_none() {
            *slot = Some(Session::open(input).map_err(|e| e.to_string())?);
        }
        return Ok("{}".into());
    }
    if command == "close" {
        if let Some(mut session) = slot.take() {
            session.finish().map_err(|e| e.to_string())?;
        }
        return Ok("{}".into());
    }
    slot.as_mut()
        .ok_or_else(|| "Recorder is not open".to_string())?
        .command(command, input)
        .map_err(|e| e.to_string())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_flochiii_stutterwatch_NativeCore_exchange(
    mut env: JNIEnv,
    _: JClass,
    command: JString,
    payload: JString,
) -> jstring {
    let result = (|| {
        let command: String = env.get_string(&command).map_err(|e| e.to_string())?.into();
        let payload: String = env.get_string(&payload).map_err(|e| e.to_string())?.into();
        if payload.len() > 1024 * 1024 {
            return Err("Native request exceeds limit".into());
        }
        dispatch(&command, &payload)
    })();
    let output = result.unwrap_or_else(|e| serde_json::json!({"error": e}).to_string());
    env.new_string(output)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
