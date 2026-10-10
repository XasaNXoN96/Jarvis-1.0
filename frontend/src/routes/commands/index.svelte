<script lang="ts">
    import { Notification, Space } from "@svelteuidev/core"
    import { InfoCircled } from "radix-icons-svelte"

    import HDivider from "@/components/elements/HDivider.svelte"
    import Footer from "@/components/Footer.svelte"
    import { appInfo, translations, translate } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    let tgLink = ""
    appInfo.subscribe(info => {
        tgLink = info.tgOfficialLink
    })
</script>

<Space h="xl" />

<Notification
    title={t('commands-wip-title')}
    icon={InfoCircled}
    color="blue"
    withCloseButton={false}
>
    {t('commands-wip-desc')}<br />
    {t('commands-wip-follow')} <a href={tgLink} target="_blank">{t('commands-wip-channel')}</a>!
</Notification>

<div class="placeholder-image">
    <img src="/media/images/tenor.gif" alt="bruh" width="320px" />
</div>

<HDivider />
<Footer />

<style>
    .placeholder-image {
        text-align: center;
        margin-top: 25px;
    }
</style>