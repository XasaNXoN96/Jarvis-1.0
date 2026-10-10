// ### STYLES
import "./css/main.scss"
import "./css/styles.scss"

// ### APP
import App from "./App.svelte"

const app = new App({
    target: document.getElementById("app")!
})

export default app
