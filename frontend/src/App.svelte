<script lang="ts">
    import { onMount, onDestroy } from "svelte"
    import { Router } from "@roxi/routify"
    import routes from "../.routify/routes.default.js"
    import { SvelteUIProvider } from "@svelteuidev/core"
    import Events from "./Events.svelte"

    import {
        loadVoiceSetting,
        loadAppInfo,
        startStatsPolling,
        stopStatsPolling,
        connectIpc,
        disconnectIpc,
        loadTranslations
    } from "@/stores"

    onMount(() => {
        // load static data
        loadVoiceSetting()
        loadAppInfo()

        // start process monitoring
        startStatsPolling(5000)

        // connect to IPC
        connectIpc()

        // load language
        loadTranslations()
    })

    onDestroy(() => {
        stopStatsPolling()
        disconnectIpc()
    })
</script>

<SvelteUIProvider themeObserver="dark" withNormalizeCSS withGlobalStyles>
    <Router {routes} />
</SvelteUIProvider>

<Events />
