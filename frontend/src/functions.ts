import { invoke } from "@tauri-apps/api/core"

// ### UTILITY FUNCTIONS

export function capitalizeFirstLetter(str: string): string {
    if (!str) return ""
    return str.charAt(0).toUpperCase() + str.slice(1)
}

export function showInExplorer(path: string): void {
    invoke("show_in_folder", { path })
        .catch(err => console.error("failed to open explorer:", err))
}
