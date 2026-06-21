use crate::database;
use serde_json::Value;

#[tauri::command] pub fn db_all(sql: String, params: Option<Vec<Value>>) -> Result<Vec<Value>, String> { let p = params.unwrap_or_default(); database::all(&sql, &p).map_err(|e| e.to_string()) }
#[tauri::command] pub fn db_get(sql: String, params: Option<Vec<Value>>) -> Result<Option<Value>, String> { let p = params.unwrap_or_default(); database::get(&sql, &p).map_err(|e| e.to_string()) }
#[tauri::command] pub fn db_run(sql: String, params: Option<Vec<Value>>) -> Result<database::RunResult, String> { let p = params.unwrap_or_default(); database::run(&sql, &p).map_err(|e| e.to_string()) }
