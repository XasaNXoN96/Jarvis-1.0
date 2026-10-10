<script lang="ts">
    import { onMount } from "svelte"
    import { listen } from "@tauri-apps/api/event"
    import { invoke } from "@tauri-apps/api/core"
    import { assistantVoice } from "@/stores"

    let voiceVal = "jarvis-og"
    assistantVoice.subscribe(value => {
        voiceVal = value || "jarvis-og"
    })

    onMount(async () => {
        // audio playback event
        await listen<{ data: string }>("audio-play", async (event) => {
            const voice = voiceVal || "jarvis-remake"
            const filename = `sound/${voice}/${event.payload.data}.wav`

            try {
                await invoke("play_sound", { filename, sleep: true })
            } catch (err) {
                console.error("failed to play sound:", err)
            }
        })

        // assistant state events
        await listen("assistant-greet", () => {
            document.getElementById("arc-reactor")?.classList.add("active")
        })

        await listen("assistant-waiting", () => {
            document.getElementById("arc-reactor")?.classList.remove("active")
        })
    })
</script>
