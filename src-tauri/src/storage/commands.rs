use std::sync::Mutex;
use tauri::State;

use super::{ChatRecord, MessageRecord, StartTurnRecord, Storage};

fn lock_storage<'a>(
    state: &'a State<'_, Mutex<Storage>>,
) -> Result<std::sync::MutexGuard<'a, Storage>, String> {
    state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())
}

#[tauri::command]
pub fn create_chat(state: State<'_, Mutex<Storage>>, title: String) -> Result<ChatRecord, String> {
    lock_storage(&state)?.create_chat(&title)
}

#[tauri::command]
pub fn list_chats(state: State<'_, Mutex<Storage>>) -> Result<Vec<ChatRecord>, String> {
    lock_storage(&state)?.list_chats()
}

#[tauri::command]
pub fn get_chat(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
) -> Result<Option<ChatRecord>, String> {
    lock_storage(&state)?.get_chat(&chat_id)
}

#[tauri::command]
pub fn start_turn(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    rewind_message_id: Option<String>,
    user_content: String,
    prompt_content: String,
    assistant_content: String,
    assistant_model: Option<String>,
    assistant_mode: Option<String>,
) -> Result<StartTurnRecord, String> {
    lock_storage(&state)?.start_turn(
        &chat_id,
        rewind_message_id.as_deref(),
        &user_content,
        &prompt_content,
        &assistant_content,
        assistant_model.as_deref(),
        assistant_mode.as_deref(),
    )
}

#[tauri::command]
pub fn save_message(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    role: String,
    content: String,
    prompt_content: Option<String>,
    model_used: Option<String>,
    mode_used: Option<String>,
) -> Result<MessageRecord, String> {
    lock_storage(&state)?.save_message(
        &chat_id,
        &role,
        &content,
        prompt_content.as_deref(),
        model_used.as_deref(),
        mode_used.as_deref(),
    )
}

#[tauri::command]
pub fn list_messages(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
) -> Result<Vec<MessageRecord>, String> {
    lock_storage(&state)?.list_messages(&chat_id)
}

#[tauri::command]
pub fn delete_chat(state: State<'_, Mutex<Storage>>, chat_id: String) -> Result<bool, String> {
    lock_storage(&state)?.delete_chat(&chat_id)
}

#[tauri::command]
pub fn create_attachment_version(
    state: State<'_, Mutex<Storage>>,
    attachment_id: Option<String>,
    blob_sha256: String,
    file_name: String,
    extension: String,
    mime_type: String,
) -> Result<super::AttachmentVersionRecord, String> {
    lock_storage(&state)?.create_attachment_version(
        attachment_id.as_deref(),
        &blob_sha256,
        &file_name,
        &extension,
        &mime_type,
    )
}

#[tauri::command]
pub fn add_attachment_reference(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    attachment_version_id: String,
    message_id: Option<String>,
    purpose: String,
    position: i64,
) -> Result<String, String> {
    lock_storage(&state)?.add_attachment_reference(
        &chat_id,
        &attachment_version_id,
        message_id.as_deref(),
        &purpose,
        position,
    )
}

#[tauri::command]
pub fn list_message_attachments(
    state: State<'_, Mutex<Storage>>,
    message_id: String,
) -> Result<Vec<super::AttachmentReferenceRecord>, String> {
    lock_storage(&state)?.list_message_attachments(&message_id)
}

#[tauri::command]
pub fn list_composer_attachments(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
) -> Result<Vec<super::AttachmentReferenceRecord>, String> {
    lock_storage(&state)?.list_composer_attachments(&chat_id)
}

#[tauri::command]
pub fn remove_attachment_reference(
    state: State<'_, Mutex<Storage>>,
    reference_id: String,
) -> Result<bool, String> {
    lock_storage(&state)?.remove_attachment_reference(&reference_id)
}

#[tauri::command]
pub fn update_message(
    state: State<'_, Mutex<Storage>>,
    message_id: String,
    content: String,
    prompt_content: Option<String>,
    model_used: Option<String>,
    mode_used: Option<String>,
) -> Result<super::MessageRecord, String> {
    let mut storage = lock_storage(&state)?;
    storage.update_message(
        &message_id,
        &content,
        prompt_content.as_deref(),
        model_used.as_deref(),
        mode_used.as_deref(),
    )
}

#[tauri::command]
pub fn rewind_chat(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    message_id: String,
) -> Result<(), String> {
    lock_storage(&state)?.rewind_chat(&chat_id, &message_id)
}
