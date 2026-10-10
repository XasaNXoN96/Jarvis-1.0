<script lang="ts">
    import { goto } from "@roxi/routify"
    import { invoke } from "@tauri-apps/api/core"
    import { onMount } from "svelte"
    import { currentLanguage, setLanguage, translations, translate } from "@/stores"
    
    let appVersion = ""
    let commandsCount = 0

    let selectedLang = "?"
    let langDropdownOpen = false

    const languages = [
        { code: "ru", label: "RU", flag: "🇷🇺", name: "Русский" },
        { code: "en", label: "EN", flag: "🇬🇧", name: "English" },
        { code: "ua", label: "UA", flag: "🇺🇦", name: "Українська" },
    ]

    onMount(async () => {
        try {
            appVersion = await invoke<string>("get_app_version")
            commandsCount = await invoke<number>("get_commands_count")

            // load saved language
            const savedLang = await invoke<string>("db_read", { key: "language" })
            if (savedLang) {
                selectedLang = savedLang
            }
        } catch {
            commandsCount = 0
        }
    })

    async function selectLanguage(code: string) {
        await setLanguage(code)
        langDropdownOpen = false
    }

    function toggleLangDropdown() {
        langDropdownOpen = !langDropdownOpen
    }

    function closeLangDropdown(e: MouseEvent) {
        const target = e.target as HTMLElement
        if (!target.closest('.lang-selector')) {
            langDropdownOpen = false
        }
    }

    $: currentLang = languages.find(l => l.code === $currentLanguage) || languages[0]
    $: t = (key: string) => translate($translations, key)
</script>

<svelte:window on:click={closeLangDropdown} />

<header id="header" class="header">
    <div class="header-left">
        <div class="logo">
            <a href="/" title="JARVIS">
                <img src="/media/128x128.png" alt="Jarvis Logo" />
            </a>
            <div class="logo-text">
                <span class="logo-title"><a href="/" id="jarvis-logo">&nbsp;</a></span>
                <span class="logo-version"><small>v</small>{appVersion} <span class="v-badge">BETA</span></span>
            </div>
        </div>
    </div>
    
    <div class="header-right">
        <button class="header-btn" on:click={() => $goto('/commands')}>
            <span class="btn-text">{t('header-commands')}</span>
            <span class="btn-badge purple">{commandsCount}+</span>
        </button>
        
        <button class="header-btn" on:click={() => $goto('/settings')}>
            <span class="btn-text">{t('header-settings')}</span>
        </button>

        <div class="lang-selector">
            <button class="lang-btn" on:click|stopPropagation={toggleLangDropdown}>
                <span class="lang-flag"><img src="/media/flags/{currentLang.label}.png" width="23px" alt="{currentLang.flag}"></span>
            </button>
            
            {#if langDropdownOpen}
                <div class="lang-dropdown">
                    {#each languages as lang}
                        <button 
                            class="lang-option" 
                            class:active={lang.code === $currentLanguage}
                            on:click|stopPropagation={() => selectLanguage(lang.code)}
                        >
                            <span class="lang-flag"><img src="/media/flags/{lang.label}.png" width="20px" alt="{lang.flag}"></span>
                            <span class="lang-name">{lang.name}</span>
                        </button>
                    {/each}
                </div>
            {/if}
        </div>
    </div>
</header>

<style lang="scss">
    .lang-selector {
        position: relative;
    }

    .lang-btn {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        padding: 0.5rem 0.65rem;
        background: transparent;
        border: none;
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.7rem;
        cursor: pointer;

        &:hover {
            background: rgba(35, 50, 55, 0.7);
        }
    }

    .lang-flag {
        font-size: 0.9rem;
        line-height: 1;
    }

    .lang-code {
        font-weight: 600;
        letter-spacing: 0.5px;
    }

    .lang-arrow {
        font-size: 0.8rem;
        opacity: 0.6;
        transition: transform 0.2s ease;

        &.open {
            transform: rotate(180deg);
        }
    }

    .lang-dropdown {
        position: absolute;
        top: calc(100% + 0.35rem);
        right: 0;
        background: rgba(20, 30, 35, 0.98);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        overflow: hidden;
        z-index: 100;
        min-width: 130px;
        box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    }

    .lang-option {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        width: 100%;
        padding: 0.6rem 0.85rem;
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.75);
        font-size: 0.75rem;
        cursor: pointer;
        transition: all 0.15s ease;
        text-align: left;

        &:hover {
            background: rgba(82, 254, 254, 0.1);
            color: #ffffff;
        }

        &.active {
            background: rgba(82, 254, 254, 0.15);
            color: #52fefe;
        }
    }

    .lang-name {
        font-weight: 500;
    }
</style>