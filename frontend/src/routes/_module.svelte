<script lang="ts">
    import { Container } from "@svelteuidev/core"
    import Header from "@/components/Header.svelte"
</script>

<Container fluid id="wrapper">
    <Header />
    <main>
        <slot></slot>
    </main>
</Container>
