//! Traducciones Anthropic Messages y OpenAI Responses (LM-CLI-7/P24).
//!
//! - `POST /v1/messages`: traduce Anthropic → chat/completions y devuelve el
//!   sobre Anthropic (JSON o SSE con `message_start`, `content_block_start`,
//!   `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`).
//! - `POST /v1/responses`: subconjunto mínimo para Codex (`input` string o
//!   array con `message`/`function_call`/`function_call_output`, `instructions`,
//!   `tools`, `max_output_tokens`, `stream`) con eventos `response.created`,
//!   `response.output_item.added`, `response.output_text.delta`,
//!   `response.output_item.done`, `response.completed`.
//! - Sin `unwrap` en rutas de request: todo error devuelve JSON 4xx/5xx.

// ---------------------------------------------------------------------------
// Anthropic → OpenAI
// ---------------------------------------------------------------------------

/// Extraer texto de un `content` Anthropic (string o array de bloques).
pub fn anthropic_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|b| {
                let t = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if t == "text" {
                    b.get("text").and_then(|t| t.as_str()).map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Traducir body Anthropic Messages → body chat/completions.
/// `served_model` es el id vivo del motor (alias rewriting D-2/LM-PXY-4/5):
/// cualquier `model` pedido se reemplaza por este.
pub fn anthropic_to_openai(
    body: &serde_json::Value,
    served_model: &str,
) -> Result<serde_json::Value, String> {
    let mut messages: Vec<serde_json::Value> = Vec::new();

    // system (string o array de bloques) → mensaje system.
    if let Some(sys) = body.get("system") {
        let text = anthropic_text(sys);
        if !text.is_empty() {
            messages.push(serde_json::json!({"role": "system", "content": text}));
        }
    }

    let arr = body
        .get("messages")
        .and_then(|m| m.as_array())
        .ok_or_else(|| "Falta el campo 'messages'".to_string())?;
    if arr.is_empty() {
        return Err("El campo 'messages' está vacío".to_string());
    }

    for m in arr {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let open_role = match role {
            "assistant" => "assistant",
            _ => "user",
        };
        let content = m.get("content").cloned().unwrap_or(serde_json::Value::Null);
        match content {
            serde_json::Value::String(s) => {
                messages.push(serde_json::json!({"role": open_role, "content": s}));
            }
            serde_json::Value::Array(blocks) => {
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<serde_json::Value> = Vec::new();
                let mut tool_results: Vec<serde_json::Value> = Vec::new();
                for b in &blocks {
                    let t = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match t {
                        "text" => {
                            if let Some(s) = b.get("text").and_then(|s| s.as_str()) {
                                text_parts.push(s.to_string());
                            }
                        }
                        "image" => {
                            // Pasar como content-part de imagen si trae `source` con datos.
                            // Forma mínima honesta: conservar el bloque para el motor.
                            if let Some(src) = b.get("source") {
                                text_parts.push(format!("[imagen: {}]", src));
                            }
                        }
                        "tool_use" => {
                            let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let input = b.get("input").cloned().unwrap_or(serde_json::json!({}));
                            let args = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
                            tool_calls.push(serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {"name": name, "arguments": args}
                            }));
                        }
                        "tool_result" => {
                            let tool_use_id = b.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                            let text = b
                                .get("content")
                                .map(anthropic_text)
                                .filter(|s| !s.is_empty())
                                .unwrap_or_default();
                            tool_results.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": text
                            }));
                        }
                        _ => {}
                    }
                }
                if open_role == "assistant" && (!tool_calls.is_empty() || !text_parts.is_empty()) {
                    let mut msg = serde_json::json!({
                        "role": "assistant",
                        "content": if text_parts.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(text_parts.join("")) }
                    });
                    if !tool_calls.is_empty() {
                        msg["tool_calls"] = serde_json::Value::Array(tool_calls);
                    }
                    messages.push(msg);
                } else if open_role == "user" && !tool_results.is_empty() {
                    // tool_result de Anthropic (rol user) → mensajes rol tool.
                    if !text_parts.is_empty() {
                        messages.push(serde_json::json!({"role": "user", "content": text_parts.join("")}));
                    }
                    messages.extend(tool_results);
                } else if !text_parts.is_empty() {
                    messages.push(serde_json::json!({"role": open_role, "content": text_parts.join("")}));
                }
            }
            _ => {}
        }
    }

    let mut out = serde_json::json!({
        "model": served_model,
        "messages": messages,
    });
    if let Some(v) = body.get("max_tokens").and_then(|v| v.as_u64()) {
        out["max_tokens"] = serde_json::json!(v);
    }
    if let Some(v) = body.get("temperature") {
        out["temperature"] = v.clone();
    }
    if let Some(v) = body.get("top_p") {
        out["top_p"] = v.clone();
    }
    if let Some(v) = body.get("stop_sequences") {
        out["stop"] = v.clone();
    }
    // tools Anthropic → tools OpenAI.
    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        let mut ot: Vec<serde_json::Value> = Vec::new();
        for t in tools {
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let schema = t.get("input_schema").cloned().unwrap_or(serde_json::json!({"type":"object"}));
            ot.push(serde_json::json!({
                "type": "function",
                "function": {"name": name, "description": desc, "parameters": schema}
            }));
        }
        out["tools"] = serde_json::Value::Array(ot);
    }
    if body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
        out["stream"] = serde_json::Value::Bool(true);
    }
    Ok(out)
}

/// Sobre Anthropic no-streaming desde una respuesta chat/completions.
pub fn openai_to_anthropic(
    resp: &serde_json::Value,
    req_model: &str,
) -> serde_json::Value {
    let choice = resp
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let msg = choice.get("message").cloned().unwrap_or(serde_json::Value::Null);
    let text = msg.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let mut content: Vec<serde_json::Value> = Vec::new();
    if !text.is_empty() {
        content.push(serde_json::json!({"type": "text", "text": text}));
    }
    if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
        for tc in calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let f = tc.get("function").cloned().unwrap_or(serde_json::Value::Null);
            let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args_str = f.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
            let input: serde_json::Value =
                serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
            content.push(serde_json::json!({
                "type": "tool_use", "id": id, "name": name, "input": input
            }));
        }
    }
    let finish = choice.get("finish_reason").and_then(|v| v.as_str()).unwrap_or("stop");
    let stop_reason = match finish {
        "tool_calls" => "tool_use",
        "length" => "max_tokens",
        _ => "end_turn",
    };
    let usage_in = resp
        .get("usage")
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let usage_out = resp
        .get("usage")
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let id = resp
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("msg_{}", rand_hex8()));
    serde_json::json!({
        "type": "message",
        "id": id,
        "role": "assistant",
        "model": req_model,
        "content": content,
        "stop_reason": stop_reason,
        "usage": {"input_tokens": usage_in, "output_tokens": usage_out}
    })
}

/// Error forma Anthropic.
pub fn anthropic_error(status: u16, err_type: &str, message: &str) -> (u16, String) {
    (
        status,
        serde_json::json!({"type": "error", "error": {"type": err_type, "message": message}}).to_string(),
    )
}

/// Error forma Responses (Codex): `{"error":{"message":...,"type":"api_error"}}`.
pub fn responses_error(status: u16, message: &str) -> (u16, String) {
    (
        status,
        serde_json::json!({"error": {"message": message, "type": "api_error"}}).to_string(),
    )
}

fn rand_hex8() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let rs = RandomState::new();
    let mut h = rs.build_hasher();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    h.write_u64(nanos ^ (std::process::id() as u64));
    format!("{:016x}", h.finish())
}

// ---------------------------------------------------------------------------
// Chat SSE → Anthropic SSE
// ---------------------------------------------------------------------------
/// Partir un buffer SSE en eventos por líneas en blanco (sin copiar de más:
/// devuelve rebanadas lógicas como Strings por evento).
/// Un evento termina en `\n\n` (o `\r\n\r\n`); el resto parcial se devuelve aparte.
pub fn split_sse_events(buffered: &str) -> Vec<String> {
    let norm = buffered.replace("\r\n", "\n");
    norm.split("\n\n")
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect()
}

/// Estado del traductor SSE (un request).
pub struct AnthropicSse {
    pub msg_id: String,
    pub req_model: String,
    pub sent_start: bool,
    pub text_open: bool,
    pub text_index: i64,
    /// tool_calls en curso: índice OpenAI → (id, name, block_index Anthropic, json_acum).
    pub tools: Vec<(String, String, i64, String)>,
    pub next_index: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub stop_reason: String,
}

impl AnthropicSse {
    pub fn new(req_model: &str, msg_id: String) -> Self {
        Self {
            msg_id,
            req_model: req_model.to_string(),
            sent_start: false,
            text_open: false,
            text_index: -1,
            tools: Vec::new(),
            next_index: 0,
            input_tokens: 0,
            output_tokens: 0,
            stop_reason: "end_turn".to_string(),
        }
    }

    fn sse(event: &str, data: &serde_json::Value) -> String {
        format!("event: {}\ndata: {}\n\n", event, data)
    }

    pub fn preamble(&mut self) -> String {
        self.sent_start = true;
        let mut s = String::new();
        s.push_str(&Self::sse(
            "message_start",
            &serde_json::json!({"type":"message_start","message":{"type":"message","id":self.msg_id,"role":"assistant","model":self.req_model,"content":[],"stop_reason":null,"usage":{"input_tokens":0,"output_tokens":0}}}),
        ));
        s
    }

    fn ensure_text(&mut self, out: &mut String) {
        if !self.text_open {
            self.text_index = self.next_index;
            self.next_index += 1;
            self.text_open = true;
            out.push_str(&Self::sse(
                "content_block_start",
                &serde_json::json!({"type":"content_block_start","index":self.text_index,"content_block":{"type":"text","text":""}}),
            ));
        }
    }

    fn close_text(&mut self, out: &mut String) {
        if self.text_open {
            out.push_str(&Self::sse(
                "content_block_stop",
                &serde_json::json!({"type":"content_block_stop","index":self.text_index}),
            ));
            self.text_open = false;
            self.text_index = -1;
        }
    }

    /// Traducir un objeto `data:` ya parseado de chat SSE. Devuelve eventos SSE.
    pub fn feed_value(&mut self, v: &serde_json::Value) -> String {
        let mut out = String::new();
        if let Some(u) = v.get("usage") {
            if let Some(p) = u.get("prompt_tokens").and_then(|x| x.as_u64()) {
                self.input_tokens = p;
            }
            if let Some(c) = u.get("completion_tokens").and_then(|x| x.as_u64()) {
                self.output_tokens = c;
            }
        }
        let choice = v
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        if let Some(fr) = choice.get("finish_reason").and_then(|x| x.as_str()) {
            self.stop_reason = match fr {
                "tool_calls" => "tool_use".to_string(),
                "length" => "max_tokens".to_string(),
                _ => "end_turn".to_string(),
            };
        }
        let delta = choice.get("delta").cloned().unwrap_or(serde_json::Value::Null);
        if let Some(t) = delta.get("content").and_then(|x| x.as_str()) {
            if !t.is_empty() {
                self.ensure_text(&mut out);
                out.push_str(&Self::sse(
                    "content_block_delta",
                    &serde_json::json!({"type":"content_block_delta","index":self.text_index,"delta":{"type":"text_delta","text":t}}),
                ));
            }
        }
        if let Some(tcs) = delta.get("tool_calls").and_then(|x| x.as_array()) {
            // Cerrar texto antes de abrir bloques tool_use.
            self.close_text(&mut out);
            for tc in tcs {
                let idx = tc.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                let id = tc.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let f = tc.get("function").cloned().unwrap_or(serde_json::Value::Null);
                let name = f.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let args = f.get("arguments").and_then(|x| x.as_str()).unwrap_or("").to_string();
                while self.tools.len() <= idx {
                    self.tools.push((String::new(), String::new(), -1, String::new()));
                }
                if !id.is_empty() {
                    self.tools[idx].0 = id;
                }
                if !name.is_empty() {
                    self.tools[idx].1 = name;
                }
                if self.tools[idx].2 < 0 {
                    let bi = self.next_index;
                    self.next_index += 1;
                    self.tools[idx].2 = bi;
                    out.push_str(&Self::sse(
                        "content_block_start",
                        &serde_json::json!({"type":"content_block_start","index":bi,"content_block":{"type":"tool_use","id":self.tools[idx].0,"name":self.tools[idx].1,"input":{}}}),
                    ));
                }
                if !args.is_empty() {
                    self.tools[idx].3.push_str(&args);
                    out.push_str(&Self::sse(
                        "content_block_delta",
                        &serde_json::json!({"type":"content_block_delta","index":self.tools[idx].2,"delta":{"type":"input_json_delta","partial_json":args}}),
                    ));
                }
            }
        }
        out
    }

    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        self.close_text(&mut out);
        for t in &self.tools {
            if t.2 >= 0 {
                out.push_str(&Self::sse(
                    "content_block_stop",
                    &serde_json::json!({"type":"content_block_stop","index":t.2}),
                ));
            }
        }
        out.push_str(&Self::sse(
            "message_delta",
            &serde_json::json!({"type":"message_delta","delta":{"stop_reason":self.stop_reason},"usage":{"input_tokens":self.input_tokens,"output_tokens":self.output_tokens}}),
        ));
        out.push_str(&Self::sse(
            "message_stop",
            &serde_json::json!({"type":"message_stop"}),
        ));
        out
    }

    /// Traducir un stream SSE crudo de chat/completions → SSE Anthropic completo.
    /// (Rama no-streaming / tests: equivale a alimentar evento por evento y cerrar.)
    #[allow(dead_code)]
    pub fn translate_stream(raw: &str, req_model: &str, msg_id: String) -> String {
        let mut st = AnthropicSse::new(req_model, msg_id);
        let mut out = st.preamble();
        for ev in split_sse_events(raw) {
            out.push_str(&st.feed_event(&ev));
        }
        out.push_str(&st.finish());
        out
    }

    /// Alimentar UN evento SSE upstream (un bloque `data: ...` ya recortado).
    /// Devuelve cero o más eventos Anthropic listos para enviar ya.
    /// El cierre (`message_delta` + `message_stop`) solo sale de `finish()`.
    pub fn feed_event(&mut self, event_block: &str) -> String {
        let mut out = String::new();
        for line in event_block.lines() {
            let line = line.trim();
            if !line.starts_with("data:") {
                continue;
            }
            let payload = line["data:".len()..].trim();
            if payload == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
                out.push_str(&self.feed_value(&v));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Responses API (Codex)
// ---------------------------------------------------------------------------

/// Traducir body Responses → body chat/completions (subconjunto Codex).
pub fn responses_to_openai(
    body: &serde_json::Value,
    served_model: &str,
) -> Result<(serde_json::Value, bool), String> {
    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if let Some(instr) = body.get("instructions").and_then(|v| v.as_str()) {
        if !instr.is_empty() {
            messages.push(serde_json::json!({"role":"system","content":instr}));
        }
    }
    match body.get("input") {
        Some(serde_json::Value::String(s)) => {
            messages.push(serde_json::json!({"role":"user","content":s}));
        }
        Some(serde_json::Value::Array(items)) => {
            for it in items {
                let t = it.get("type").and_then(|v| v.as_str()).unwrap_or("message");
                match t {
                    "message" => {
                        let role = it.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                        let open_role = match role {
                            "assistant" => "assistant",
                            "system" | "developer" => "system",
                            _ => "user",
                        };
                        let text = it
                            .get("content")
                            .map(anthropic_text)
                            .filter(|s| !s.is_empty())
                            .unwrap_or_default();
                        // content puede ser array de parts {type:input_text|output_text}.
                        let text2 = if text.is_empty() {
                            if let Some(arr) = it.get("content").and_then(|c| c.as_array()) {
                                arr.iter()
                                    .filter_map(|p| {
                                        let pt = p.get("type").and_then(|x| x.as_str()).unwrap_or("");
                                        if pt == "input_text" || pt == "output_text" || pt == "text" {
                                            p.get("text").and_then(|x| x.as_str()).map(str::to_string)
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("")
                            } else {
                                String::new()
                            }
                        } else {
                            text
                        };
                        messages.push(serde_json::json!({"role":open_role,"content":text2}));
                    }
                    "function_call" => {
                        let call_id = it.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                        let name = it.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args = it.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
                        messages.push(serde_json::json!({
                            "role":"assistant","content":null,
                            "tool_calls":[{"id":call_id,"type":"function","function":{"name":name,"arguments":args}}]
                        }));
                    }
                    "function_call_output" => {
                        let call_id = it.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                        let output = it.get("output").map(|o| match o {
                            serde_json::Value::String(s) => s.clone(),
                            _ => o.to_string(),
                        }).unwrap_or_default();
                        messages.push(serde_json::json!({
                            "role":"tool","tool_call_id":call_id,"content":output
                        }));
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    if messages.is_empty() {
        return Err("Falta el campo 'input'".to_string());
    }
    let mut out = serde_json::json!({"model": served_model, "messages": messages});
    if let Some(v) = body.get("max_output_tokens").and_then(|v| v.as_u64()) {
        out["max_tokens"] = serde_json::json!(v);
    }
    if let Some(v) = body.get("temperature") {
        out["temperature"] = v.clone();
    }
    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        let mut ot: Vec<serde_json::Value> = Vec::new();
        for t in tools {
            // Responses function tool: {type:function,name,description?,parameters?}
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let params = t.get("parameters").cloned().unwrap_or(serde_json::json!({"type":"object"}));
            ot.push(serde_json::json!({
                "type":"function","function":{"name":name,"description":desc,"parameters":params}
            }));
        }
        out["tools"] = serde_json::Value::Array(ot);
    }
    if stream {
        out["stream"] = serde_json::Value::Bool(true);
    }
    // Modelo pedido siempre se reescribe al servido (alias rewriting).
    if let Some(m) = body.get("model").and_then(|v| v.as_str()) {
        let _ = m;
    }
    Ok((out, stream))
}

/// Sobre Responses no-streaming desde chat/completions.
pub fn openai_to_responses(resp: &serde_json::Value, req_model: &str) -> serde_json::Value {
    let choice = resp
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let msg = choice.get("message").cloned().unwrap_or(serde_json::Value::Null);
    let text = msg.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let mut output: Vec<serde_json::Value> = Vec::new();
    if !text.is_empty() {
        output.push(serde_json::json!({
            "type":"message","role":"assistant",
            "content":[{"type":"output_text","text":text}]
        }));
    }
    if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
        for tc in calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let f = tc.get("function").cloned().unwrap_or(serde_json::Value::Null);
            let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = f.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
            output.push(serde_json::json!({
                "type":"function_call","call_id":id,"name":name,"arguments":args
            }));
        }
    }
    let usage_in = resp.get("usage").and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
    let usage_out = resp.get("usage").and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
    let id = resp.get("id").and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| format!("resp_{}", rand_hex8()));
    serde_json::json!({
        "id": id,
        "object": "response",
        "model": req_model,
        "output": output,
        "usage": {"input_tokens": usage_in, "output_tokens": usage_out}
    })
}

/// Traductor incremental Responses (un request con `stream:true`).
pub struct ResponsesSse {
    pub req_model: String,
    pub resp_id: String,
    pub calls: Vec<(String, String, String)>,
    pub usage_in: u64,
    pub usage_out: u64,
    pub finish: String,
    pub announced: Vec<bool>,
}

fn resp_sse(event: &str, data: &serde_json::Value) -> String {
    format!("event: {}\ndata: {}\n\n", event, data)
}

impl ResponsesSse {
    pub fn new(req_model: &str, resp_id: String) -> Self {
        Self {
            req_model: req_model.to_string(),
            resp_id,
            calls: Vec::new(),
            usage_in: 0,
            usage_out: 0,
            finish: "completed".to_string(),
            announced: Vec::new(),
        }
    }

    pub fn preamble(&self) -> String {
        let mut out = String::new();
        out.push_str(&resp_sse("response.created", &serde_json::json!({"type":"response.created","response":{"id":self.resp_id,"model":self.req_model,"status":"in_progress"}})));
        out.push_str(&resp_sse("response.output_item.added", &serde_json::json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","role":"assistant","content":[]}})));
        out
    }

    /// Alimentar UN evento SSE upstream. Devuelve eventos listos ya
    /// (`response.output_text.delta` inmediatos; los `function_call` salen en `finish()`).
    pub fn feed_event(&mut self, event_block: &str) -> String {
        let mut out = String::new();
        for line in event_block.lines() {
            let line = line.trim();
            if !line.starts_with("data:") {
                continue;
            }
            let payload = line["data:".len()..].trim();
            if payload == "[DONE]" {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(payload) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(u) = v.get("usage") {
                if let Some(p) = u.get("prompt_tokens").and_then(|x| x.as_u64()) {
                    self.usage_in = p;
                }
                if let Some(c) = u.get("completion_tokens").and_then(|x| x.as_u64()) {
                    self.usage_out = c;
                }
            }
            let choice = v.get("choices").and_then(|c| c.as_array()).and_then(|a| a.first()).cloned().unwrap_or(serde_json::Value::Null);
            if choice.get("finish_reason").and_then(|x| x.as_str()) == Some("length") {
                self.finish = "incomplete".to_string();
            }
            let delta = choice.get("delta").cloned().unwrap_or(serde_json::Value::Null);
            if let Some(t) = delta.get("content").and_then(|x| x.as_str()) {
                if !t.is_empty() {
                    out.push_str(&resp_sse("response.output_text.delta", &serde_json::json!({"type":"response.output_text.delta","output_index":0,"delta":t})));
                }
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(|x| x.as_array()) {
                for tc in tcs {
                    let idx = tc.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                    let id = tc.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let f = tc.get("function").cloned().unwrap_or(serde_json::Value::Null);
                    let name = f.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let args = f.get("arguments").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    while self.calls.len() <= idx {
                        self.calls.push((String::new(), String::new(), String::new()));
                        self.announced.push(false);
                    }
                    if !id.is_empty() {
                        self.calls[idx].0 = id;
                    }
                    if !name.is_empty() {
                        self.calls[idx].1 = name;
                    }
                    if !args.is_empty() {
                        self.calls[idx].2.push_str(&args);
                    }
                    // Anunciar el item function_call en cuanto se conoce id/nombre.
                    if !self.announced[idx] && (!self.calls[idx].0.is_empty() || !self.calls[idx].1.is_empty()) {
                        self.announced[idx] = true;
                        out.push_str(&resp_sse("response.output_item.added", &serde_json::json!({"type":"response.output_item.added","output_index":idx+1,"item":{"type":"function_call","call_id":self.calls[idx].0,"name":self.calls[idx].1,"arguments":""}})));
                    }
                }
            }
        }
        out
    }

    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        for (i, (call_id, name, args)) in self.calls.iter().enumerate() {
            if call_id.is_empty() && name.is_empty() {
                continue;
            }
            // Si ya se anunció el added incremental, solo cerrar con done.
            if !self.announced[i] {
                out.push_str(&resp_sse("response.output_item.added", &serde_json::json!({"type":"response.output_item.added","output_index":i+1,"item":{"type":"function_call","call_id":call_id,"name":name,"arguments":""}})));
            }
            out.push_str(&resp_sse("response.output_item.done", &serde_json::json!({"type":"response.output_item.done","output_index":i+1,"item":{"type":"function_call","call_id":call_id,"name":name,"arguments":args}})));
        }
        out.push_str(&resp_sse("response.output_item.done", &serde_json::json!({"type":"response.output_item.done","output_index":0,"item":{"type":"message","role":"assistant","content":[]}})));
        out.push_str(&resp_sse("response.completed", &serde_json::json!({"type":"response.completed","response":{"id":self.resp_id,"model":self.req_model,"status":self.finish,"usage":{"input_tokens":self.usage_in,"output_tokens":self.usage_out}}})));
        out
    }
}

/// Traducir stream SSE crudo de chat → eventos SSE Responses (Codex).
/// (Equivale a alimentar evento por evento y cerrar; se conserva para tests.)
#[allow(dead_code)]
pub fn responses_translate_stream(raw: &str, req_model: &str, resp_id: &str) -> String {
    let mut st = ResponsesSse::new(req_model, resp_id.to_string());
    let mut out = st.preamble();
    for ev in split_sse_events(raw) {
        out.push_str(&st.feed_event(&ev));
    }
    out.push_str(&st.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_tool_use_tool_result_roundtrip() {
        let req = serde_json::json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 128,
            "system": "Eres útil.",
            "tools": [{"name": "get_weather", "description": "Clima", "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}}}],
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "¿clima en Madrid?"}]},
                {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {"city": "Madrid"}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "Soleado 24C"}]}
            ]
        });
        let open = anthropic_to_openai(&req, "served-id").unwrap();
        assert_eq!(open["model"], serde_json::json!("served-id"));
        let msgs = open["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], serde_json::json!("system"));
        // assistant con tool_calls
        let asst = msgs.iter().find(|m| m["role"] == "assistant").unwrap();
        assert_eq!(asst["tool_calls"][0]["id"], serde_json::json!("toolu_1"));
        assert_eq!(asst["tool_calls"][0]["function"]["name"], serde_json::json!("get_weather"));
        // tool_result → rol tool
        let tool = msgs.iter().find(|m| m["role"] == "tool").unwrap();
        assert_eq!(tool["tool_call_id"], serde_json::json!("toolu_1"));
        assert!(tool["content"].as_str().unwrap().contains("Soleado"));
        // tools mapeadas
        assert_eq!(open["tools"][0]["function"]["name"], serde_json::json!("get_weather"));
    }

    #[test]
    fn incremental_anthropic_delta_antes_del_cierre() {
        let mut st = AnthropicSse::new("req-model", "msg_inc".to_string());
        let mut out = st.preamble();
        // Primer evento: delta de texto → debe salir content_block_delta ya,
        // sin message_delta/message_stop todavía.
        out.push_str(&st.feed_event("data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"},\"finish_reason\":null}]}\n\n"));
        assert!(out.contains("content_block_delta"), "delta inmediato: {}", out);
        assert!(!out.contains("message_delta"), "cierre prematuro: {}", out);
        assert!(!out.contains("message_stop"), "cierre prematuro: {}", out);
        // Cierre con usage → ahora sí message_delta con usage y message_stop.
        out.push_str(&st.feed_event("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":7}}\n\ndata: [DONE]\n\n"));
        assert!(!out.contains("message_stop"), "finish() aún no llamado: {}", out);
        out.push_str(&st.finish());
        let seq: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("event: "))
            .map(|l| &l[7..])
            .collect();
        assert_eq!(
            seq,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        assert!(out.contains("\"output_tokens\":7") || out.contains("\"output_tokens\": 7"));
    }

    #[test]
    fn incremental_responses_delta_antes_del_completed() {
        let mut st = ResponsesSse::new("req-model", "resp_inc".to_string());
        let mut out = st.preamble();
        out.push_str(&st.feed_event("data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"},\"finish_reason\":null}]}\n\n"));
        assert!(out.contains("response.output_text.delta"), "delta inmediato: {}", out);
        assert!(!out.contains("response.completed"), "cierre prematuro: {}", out);
        out.push_str(&st.feed_event("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":4}}\n\ndata: [DONE]\n\n"));
        assert!(!out.contains("response.completed"), "finish() aún no llamado: {}", out);
        out.push_str(&st.finish());
        let seq: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("event: "))
            .map(|l| &l[7..])
            .collect();
        assert!(seq.first() == Some(&"response.created"));
        assert!(seq.contains(&"response.output_text.delta"));
        assert!(seq.last() == Some(&"response.completed"));
        assert!(out.contains("\"output_tokens\":4") || out.contains("\"output_tokens\": 4"));
    }

    #[test]
    fn chat_sse_a_anthropic_sse_exacta() {
        let raw = "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"Hola\"},\"finish_reason\":null}]}\n\n\
                   data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\" mundo\"},\"finish_reason\":null}]}\n\n\
                   data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":7}}\n\n\
                   data: [DONE]\n\n";
        let out = AnthropicSse::translate_stream(raw, "req-model", "msg_test".to_string());
        let seq: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("event: "))
            .map(|l| &l[7..])
            .collect();
        assert_eq!(
            seq,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        assert!(out.contains("\"output_tokens\":7") || out.contains("\"output_tokens\": 7"));
        assert!(out.contains("message_stop"));
    }

    #[test]
    fn chat_sse_tool_calls_a_input_json_delta() {
        let raw = "data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"function\":{\"name\":\"sh\",\"arguments\":\"{\\\"cmd\\\"\"}}]},\"finish_reason\":null}]}\n\n\
                   data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\":\\\"ls\\\"}\"}}]},\"finish_reason\":null}]}\n\n\
                   data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":20}}\n\n\
                   data: [DONE]\n\n";
        let out = AnthropicSse::translate_stream(raw, "req-model", "msg_tools".to_string());
        let seq: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("event: "))
            .map(|l| &l[7..])
            .collect();
        assert_eq!(
            seq,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        // Los deltas de herramienta son input_json_delta y el stop_reason es tool_use.
        assert!(out.contains("input_json_delta"));
        assert!(out.contains("\"stop_reason\":\"tool_use\""));
        assert!(out.contains("\"output_tokens\":20") || out.contains("\"output_tokens\": 20"));
    }
    #[test]
    fn responses_mapea_mensaje_y_function_call_output() {
        let req = serde_json::json!({
            "model": "codex-pedido",
            "instructions": "Sé breve.",
            "input": [
                {"type": "message", "role": "user", "content": "hola"},
                {"type": "function_call", "call_id": "call_1", "name": "sh", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "ok"}
            ]
        });
        let (open, stream) = responses_to_openai(&req, "served-id").unwrap();
        assert!(!stream);
        assert_eq!(open["model"], serde_json::json!("served-id"));
        let msgs = open["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], serde_json::json!("system"));
        let tool = msgs.iter().find(|m| m["role"] == "tool").unwrap();
        assert_eq!(tool["tool_call_id"], serde_json::json!("call_1"));
    }
}
