use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Emitter;
use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, AddBos, LlamaModel},
    sampling::LlamaSampler,
};

use crate::ProgressPayload;

const LLAMA_MODEL_URL: &str = "https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf";
const LLAMA_MODEL_FILENAME: &str = "Qwen3-8B-Q4_K_M.gguf";
const TRANSLATE_BATCH_SIZE: usize = 30;

struct LlamaState {
    backend: LlamaBackend,
    model: LlamaModel,
}

static LLAMA_STATE: Mutex<Option<LlamaState>> = Mutex::new(None);

pub fn llama_model_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/jusur")
        .join(LLAMA_MODEL_FILENAME)
}

fn run_llama_inference(prompt: &str) -> Result<String, String> {
    let model_path = llama_model_path();

    let mut state_guard = LLAMA_STATE.lock().unwrap();
    if state_guard.is_none() {
        println!("[llama] Loading model: {}", model_path.display());
        let backend = LlamaBackend::init()
            .map_err(|e| format!("Failed to init llama backend: {e}"))?;
        let model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default())
            .map_err(|e| format!("Failed to load LLM model: {e}"))?;
        *state_guard = Some(LlamaState { backend, model });
        println!("[llama] Model loaded.");
    }

    let state = state_guard.as_ref().unwrap();

    // Qwen3 chat format with /no_think to disable reasoning
    let formatted = format!(
        "<|im_start|>system\n/no_think<|im_end|>\n<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n"
    );

    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(std::num::NonZeroU32::new(2048));
    let mut ctx = state.model
        .new_context(&state.backend, ctx_params)
        .map_err(|e| format!("Failed to create LLM context: {e}"))?;

    let tokens = state.model
        .str_to_token(&formatted, AddBos::Never)
        .map_err(|e| format!("Failed to tokenize prompt: {e}"))?;

    let mut batch = LlamaBatch::new(tokens.len().max(512), 1);
    let last_idx = tokens.len() as i32 - 1;
    for (i, token) in (0_i32..).zip(tokens.into_iter()) {
        batch.add(token, i, &[0], i == last_idx)
            .map_err(|e| format!("Failed to add token to batch: {e}"))?;
    }
    ctx.decode(&mut batch).map_err(|e| format!("LLM decode failed: {e}"))?;

    let mut n_cur = batch.n_tokens();
    let n_max = n_cur + 1024;
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut sampler = LlamaSampler::greedy();
    let mut output = String::new();

    while n_cur <= n_max {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if token == state.model.token_eos() {
            break;
        }

        let piece = state.model
            .token_to_piece(token, &mut decoder, false, None)
            .map_err(|e| format!("token_to_piece failed: {e}"))?;
        output.push_str(&piece);

        batch.clear();
        batch.add(token, n_cur, &[0], true)
            .map_err(|e| format!("Failed to add token: {e}"))?;
        ctx.decode(&mut batch).map_err(|e| format!("LLM decode failed: {e}"))?;

        n_cur += 1;
    }

    // Strip <think>...</think> blocks that Qwen3 may still emit
    let mut s = output;
    while let Some(start_pos) = s.find("<think>") {
        if let Some(end_pos) = s.find("</think>") {
            s = format!("{}{}", &s[..start_pos], &s[end_pos + 8..]);
        } else {
            s = s[..start_pos].to_string();
            break;
        }
    }

    Ok(s)
}

async fn translate_batch(batch_idx: usize, total_batches: usize, lines: &[(usize, String)]) -> Result<Vec<String>, String> {
    let numbered: Vec<String> = lines
        .iter()
        .map(|(i, text)| format!("{}. {}", i + 1, text))
        .collect();

    let prompt = format!(
        "Translate each numbered line from Arabic to English. Return ONLY the numbered translations, one per line. Do not add any explanation or extra text.\n\n{}",
        numbered.join("\n")
    );

    println!("[translate] Batch {}/{}: running inference ({} chars)...", batch_idx, total_batches, prompt.len());
    let start = std::time::Instant::now();

    let content = tokio::task::spawn_blocking(move || run_llama_inference(&prompt))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    let elapsed = start.elapsed();
    println!("[translate] Batch {}/{}: done in {:.1}s", batch_idx, total_batches, elapsed.as_secs_f64());
    println!("[translate] Batch {}/{}: raw response:\n{}", batch_idx, total_batches, content);

    let mut translations: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        if let Some(pos) = trimmed.find(|c: char| c == '.' || c == ')') {
            let num_part = &trimmed[..pos];
            if num_part.trim().parse::<usize>().is_ok() {
                translations.push(trimmed[pos + 1..].trim().to_string());
            }
        }
    }

    println!("[translate] Batch {}/{}: parsed {} translations for {} lines", batch_idx, total_batches, translations.len(), lines.len());
    Ok(translations)
}

pub async fn translate_segments(segments: &[serde_json::Value]) -> Result<Vec<serde_json::Value>, String> {
    if segments.is_empty() {
        return Ok(vec![]);
    }

    let total_start = std::time::Instant::now();

    let lines: Vec<(usize, String)> = segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let text = seg.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
            (i, text)
        })
        .collect();

    let total_batches = (lines.len() + TRANSLATE_BATCH_SIZE - 1) / TRANSLATE_BATCH_SIZE;
    println!("[translate] Starting translation: {} segments in {} batches of {}", segments.len(), total_batches, TRANSLATE_BATCH_SIZE);

    let mut all_translations: Vec<String> = Vec::with_capacity(segments.len());

    for (batch_idx, chunk) in lines.chunks(TRANSLATE_BATCH_SIZE).enumerate() {
        let batch_translations = translate_batch(batch_idx + 1, total_batches, chunk).await?;

        for (j, (_, original)) in chunk.iter().enumerate() {
            let translation = batch_translations
                .get(j)
                .cloned()
                .unwrap_or_else(|| original.clone());
            all_translations.push(translation);
        }
    }

    println!("[translate] All done: {} segments translated in {:.1}s", all_translations.len(), total_start.elapsed().as_secs_f64());

    let enriched: Vec<serde_json::Value> = segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let mut obj = seg.clone();
            let translation = all_translations
                .get(i)
                .cloned()
                .unwrap_or_else(|| {
                    seg.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string()
                });
            obj.as_object_mut()
                .map(|m| m.insert("translation".to_string(), serde_json::Value::String(translation)));
            obj
        })
        .collect();

    Ok(enriched)
}

#[tauri::command]
pub fn check_llama_model() -> bool {
    llama_model_path().exists()
}

#[tauri::command]
pub async fn download_llama_model(app: tauri::AppHandle) -> Result<(), String> {
    use futures_util::StreamExt;

    let model_path = llama_model_path();
    let cache_dir = model_path.parent().unwrap();
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("Failed to create cache dir: {}", e))?;

    app.emit("llama-model-progress", ProgressPayload {
        status: "downloading".into(),
        message: "Downloading LLM model...".into(),
        progress: 0.0,
        speed: None,
    }).ok();

    let response = reqwest::get(LLAMA_MODEL_URL)
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    let total_size = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    let mut last_bytes: u64 = 0;
    let mut current_speed: f64 = 0.0;

    let mut file = std::fs::File::create(&model_path)
        .map_err(|e| format!("Failed to create model file: {}", e))?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
        use std::io::Write;
        file.write_all(&chunk).map_err(|e| format!("Write error: {}", e))?;
        downloaded += chunk.len() as u64;
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(last_emit).as_secs_f64();
        if elapsed >= 0.5 {
            current_speed = (downloaded - last_bytes) as f64 / elapsed;
            last_bytes = downloaded;
            last_emit = now;
        }
        if total_size > 0 {
            let pct = (downloaded as f64 / total_size as f64) * 100.0;
            app.emit("llama-model-progress", ProgressPayload {
                status: "downloading".into(),
                message: format!("Downloading LLM model... {:.0}%", pct),
                progress: pct,
                speed: Some(current_speed),
            }).ok();
        }
    }

    app.emit("llama-model-progress", ProgressPayload {
        status: "done".into(),
        message: "LLM model ready!".into(),
        progress: 100.0,
        speed: None,
    }).ok();

    Ok(())
}
