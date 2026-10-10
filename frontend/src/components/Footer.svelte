<script lang="ts">
    import { onMount } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { appInfo, currentLanguage, translations, translate } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    let authorName = ""
    let tgLink = ""
    let repoLink = ""
    let boostyLink = ""
    let patreonLink = ""

    const currentYear = new Date().getFullYear()

    appInfo.subscribe(info => {
        tgLink = info.tgOfficialLink
        repoLink = info.repositoryLink
        boostyLink = info.boostySupportLink
        patreonLink = info.patreonSupportLink
    })

    onMount(async () => {
        try {
            authorName = await invoke<string>("get_author_name")
        } catch (err) {
            console.error("failed to get author name:", err)
        }
    })
</script>

<footer id="footer">
    <p>© {currentYear}. {t('footer-author')}: <b>{authorName}</b></p>
    <p class="links">
        {#if $currentLanguage === "ru" || $currentLanguage === "ua"}
        <a href={tgLink} target="_blank" class="telegram-link">
            <img src="/media/icons/telegram.webp" alt="Telegram" width="18px" />
            &nbsp;<span>{t('footer-telegram')}</span>
        </a>
        &nbsp;
        {/if}
        <a href={repoLink} target="_blank">
            <img src="/media/icons/github-logo.png" alt="GitHub" width="18px" />
            &nbsp;<span>{t('footer-github')}</span>
        </a>
    </p>
    <p class="links last">
        {#if $currentLanguage === "ru"}
        {t('footer-support')} <a href={boostyLink} target="_blank" class="telegram-link">
            <img src="/media/icons/boosty.webp" alt="Boosty" width="18px" />
            <span>Boosty</span>
        </a>.
        {/if}
        {#if $currentLanguage === "ua" || $currentLanguage === "en"}
        {t('footer-support')} <a href={patreonLink} target="_blank" class="telegram-link">
            <img src="/media/icons/patreon.png" alt="Patreon" width="18px" />
            <span>Patreon</span>
        </a>.
        {/if}
    </p>
</footer>

<style lang="scss">
    #footer {
        text-align: center;
        color: #6c6e71;
        font-size: 13px;
        font-weight: normal;
        line-height: 1.7em;
        margin-top: 15px;

        p {
            margin: 0;
            padding: 0;

            &.links {
                margin-top: 5px;
                margin-bottom: 15px;

                &.last {
                    margin-top: -5px;
                }
            }
        }

        a {
            color: #555759!important;
            text-decoration: none;
            transition: 0.3s;
            
            & > span {
                color: #185876;
                border-bottom: 1px solid #185876;
                transition: 0.3s;
            }

            img {
                opacity: 0.5;
                transition: opacity 0.5s;
                margin-top: -3px;
            }

            &:hover {
                color: #777a7d!important;

                & > span {
                    color: #2A9CD0;
                }

                img {
                    opacity: 1;
                }
            }

            &.telegram-link {
                color: #185876;
                display: inline-block;

                &:hover {
                    color: #2A9CD0;
                    // background: url(/media/images/bg/bg24.gif);
                    // background-repeat: no-repeat;
                    // background-size: contain;
                }
            }

            &.special-link {
                color: #941d92;
                display: inline-block;

                &:hover {
                    color: #FF07FC;
                    background: url(/media/images/bg/bg24.gif);
                    background-repeat: no-repeat;
                    background-size: contain;
                }
            }
        }
    }
</style>
