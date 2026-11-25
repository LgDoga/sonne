use std::sync::Mutex;
use tauri::Manager;

mod storage;

const CORE_SYSTEM_RULES: &str = include_str!("../../Core_System_Rules.md");

const SONNE_VOICE: &str = include_str!("../../Sonne_Voice.md");

const PRO_AI_INTELLIGENCE: &str = include_str!("../../Pro_AI_Intelligence.md");

const GEMMA_INTELLIGENCE: &str = include_str!("../../Gemma_Intelligence.md");

const PRO_AI_FINAL: &str = include_str!("../../Pro_AI_Final_Instructions.md");

const GEMMA_FINAL: &str = include_str!("../../Gemma_Final_Instructions.md");

#[tauri::command]
fn greet(name: &str) -> String {
    format!(
        "Hello, {}! You've been greeted from the Rust backend!",
        name
    )
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
        .register_asynchronous_uri_scheme_protocol("sonne-file", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            storage::handle_preview_request(app, request, responder);
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;

            let storage =
                storage::Storage::initialize(&app_data_dir).map_err(std::io::Error::other)?;

            app.manage(Mutex::new(storage));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_runtime_instructions,
            storage::commands::create_chat,
            storage::commands::list_chats,
            storage::commands::get_chat,
            storage::commands::save_message,
            storage::commands::start_turn,
            storage::commands::update_message,
            storage::commands::rewind_chat,
            storage::commands::list_messages,
            storage::commands::delete_chat,
            storage::commands::create_attachment_version,
            storage::commands::add_attachment_reference,
            storage::commands::list_message_attachments,
            storage::commands::list_composer_attachments,
            storage::commands::remove_attachment_reference,
            storage::import::import_attachments,
            storage::import::import_files,
            storage::import::import_clipboard_image,
            storage::attachments::promote_composer_attachments,
            storage::attachments::read_attachment_chunk,
            storage::archive::list_archive_entries,
            storage::archive::read_archive_entry_chunk,
            storage::processing::get_attachment_processing_status,
            storage::processing::begin_attachment_processing,
            storage::processing::save_extracted_document,
            storage::processing::mark_attachment_processing_failed,
            storage::processing::get_attachment_document_context,
            storage::processing::search_attachment_documents,
            storage::processing::get_document_embedding_batch,
            storage::processing::save_document_embedding_batch,
            storage::processing::finalize_document_embedding_index,
            storage::processing::get_document_summary_chunk_batch,
            storage::processing::clear_document_summary_stage,
            storage::processing::save_document_summary_parts,
            storage::processing::get_document_summary_parts,
            storage::processing::finalize_document_summary,
            storage::processing::get_document_summary,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
