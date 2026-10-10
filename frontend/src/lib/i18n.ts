import { writable, derived } from "svelte/store"
import { invoke } from "@tauri-apps/api/core"

// stores
export const translations = writable<Record<string, string>>({})
export const currentLanguage = writable<string>("ru")

// simple helper function (not a store)
export function translate(translations: Record<string, string>, key: string, fallback?: string): string {
    return translations[key] || fallback || key
}

// load translations from backend
export async function loadTranslations() {
    try {
        const [trans, lang] = await Promise.all([
            invoke<Record<string, string>>("get_translations"),
            invoke<string>("get_current_language")
        ])
        translations.set(trans)
        currentLanguage.set(lang)
    } catch (err) {
        console.error("Failed to load translations:", err)
    }
}

// change language
export async function setLanguage(lang: string) {
    try {
        const newTranslations = await invoke<Record<string, string>>("set_language", { lang })
        translations.set(newTranslations)
        currentLanguage.set(lang)
    } catch (err) {
        console.error("Failed to set language:", err)
    }
}

export async function loadLanguage() {
    try {
        const lang = await invoke<string>("db_read", { key: "language" })
        if (lang) {
            currentLanguage.set(lang)
        }
    } catch (err) {
        console.error("Failed to load language:", err)
    }
}

export async function getSupportedLanguages(): Promise<string[]> {
    try {
        return await invoke<string[]>("get_supported_languages")
    } catch {
        return ["ru", "en", "ua"]
    }
}