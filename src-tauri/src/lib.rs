
const CORE_SYSTEM_RULES: &str =
    include_str!("../../Core_System_Rules.md");

const SONNE_VOICE: &str =
    include_str!("../../Sonne_Voice.md");

const PRO_AI_INTELLIGENCE: &str =
    include_str!("../../Pro_AI_Intelligence.md");

const GEMMA_INTELLIGENCE: &str =
    include_str!("../../Gemma_Intelligence.md");

const PRO_AI_FINAL: &str =
    include_str!("../../Pro_AI_Final_Instructions.md");

const GEMMA_FINAL: &str =
    include_str!("../../Gemma_Final_Instructions.md");

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_runtime_instructions(model: &str) -> Result<String, String> {
    let (intelligence, final_instructions) = match model {
        "pro-ai" => (PRO_AI_INTELLIGENCE, PRO_AI_FINAL),
        "gemma" => (GEMMA_INTELLIGENCE, GEMMA_FINAL),
        _ => return Err(format!("Unknown model profile: {}", model)),
    };

    Ok([
        CORE_SYSTEM_RULES,
        SONNE_VOICE,
        intelligence,
        final_instructions,
    ]
    .join("\n\n"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            get_runtime_instructions
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
