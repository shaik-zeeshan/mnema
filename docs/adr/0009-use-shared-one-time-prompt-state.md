# Use shared one-time prompt state

Mnema will persist dismissible one-time app prompts in a shared **One-Time Prompt State** file under Tauri `app_config_dir()` rather than in recording settings, browser local storage, or prompt-specific files. Each **One-Time Prompt** uses a stable versioned prompt id with shown, dismissed, and completed timestamps, so future one-time dialogs can reuse the same app-owned persistence convention without changing capture behavior.
