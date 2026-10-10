<script lang="ts">
    import { jarvisState } from "@/stores"

    // map state to class
    $: stateClass = getStateClass($jarvisState)

    function getStateClass(state: string): string {
        switch (state) {
            case "listening":
            case "processing":
                return "active"
            case "idle":
                return "idle"
            case "disconnected":
            default:
                return "disconnected"
        }
    }
</script>

<!-- Based on: https://github.com/rembertdesigns/Iron-Man-Arc-Reactor-Pure-CSS -->
<!-- and https://codepen.io/FlyingEmu/pen/DZNqEj -->

<div class="arc-reactor-wrapper">
    <div id="arc-reactor" class="reactor-container {stateClass}">
        <div class="reactor-container-inner circle abs-center">
            <ul class="marks">
                {#each Array(60) as _, i}
                    <li></li>
                {/each}
            </ul>
            <div class="e7">
                <div class="semi_arc_3 e5_1">
                    <div class="semi_arc_3 e5_2">
                        <div class="semi_arc_3 e5_3">
                            <div class="semi_arc_3 e5_4"></div>
                        </div>
                    </div>
                </div>
            </div>
        </div>
        <div class="tunnel circle abs-center"></div>
        <div class="core-wrapper circle abs-center"></div>
        <div class="core-outer circle abs-center"></div>
        <div class="core-inner circle abs-center"></div>
        <div class="coil-container">
            {#each Array(8) as _, i}
                <div class="coil coil-{i + 1}"></div>
            {/each}
        </div>
    </div>

    <div class="state-label">
        <span class="status-dot"></span>
        <span class="label-text">
            {#if $jarvisState === "disconnected"}
                Отключен
            {:else if $jarvisState === "idle"}
                Ожидание
            {:else if $jarvisState === "listening"}
                Слушаю
            {:else if $jarvisState === "processing"}
                Обработка
            {/if}
        </span>
    </div>
</div>

<style lang="scss" global>
    // [ Variables ]--
    $arc-radius: 130px;
    $size3: 6px;
    $cshadow: rgba(2, 254, 255, 0.8);
    $marks-color-1: rgba(2, 254, 255, 1);
    $marks-color-2: rgba(2, 254, 255, 0.3);
    $colour1: rgba(2, 255, 255, 0.15);
    $colour3: rgba(2, 255, 255, 0.3);

    // [ Wrapper ]--
    .arc-reactor-wrapper {
        display: flex;
        flex-direction: column;
        align-items: center;
        padding: 1rem 0;
    }

    .state-label {
        margin-top: 0.5rem;
        font-size: 0.9rem;
        color: #52fefe;
        text-transform: uppercase;
        letter-spacing: 2px;
        opacity: 0.8;
        transition: opacity 0.3s ease;
    }

    // [ Base container ]--
    .reactor-container {
        width: 300px;
        height: 320px;
        margin: auto;
        position: relative;
        border-radius: 50%;
        transition: scale 1s ease, opacity 0.5s ease;
        scale: 0.9;
        opacity: 0.9;

        ul {
            list-style: none;
            margin: 0;
            padding: 0;
        }
    }

    .reactor-container-inner {
        height: 238px;
        width: 238px;
        background-color: #161a1b;
        box-shadow: 0px 0px 50px 15px $colour3, inset 0px 0px 50px 15px $colour3;
        transition: box-shadow 0.5s ease;
    }

    // [ Utility classes ]--
    .circle {
        border-radius: 50%;
    }

    .abs-center {
        position: absolute;
        top: 0;
        right: 0;
        bottom: 0;
        left: 0;
        margin: auto;
    }

    // [ Core elements ]--
    .core-inner {
        width: 70px;
        height: 70px;
        border: 5px solid #1b4e5f;
        background-color: #ffffff;
        box-shadow: 0px 0px 7px 5px #52fefe, 0px 0px 10px 10px #52fefe inset;
        transition: box-shadow 0.5s ease;
    }

    .core-outer {
        width: 120px;
        height: 120px;
        border: 1px solid #52fefe;
        background-color: #ffffff;
        box-shadow: 0px 0px 2px 1px #52fefe, 0px 0px 10px 5px #52fefe inset;
        transition: box-shadow 0.5s ease;
    }

    .core-wrapper {
        width: 180px;
        height: 180px;
        background-color: #073c4b;
        box-shadow: 0px 0px 5px 4px #52fefe, 0px 0px 6px 2px #52fefe inset;
        transition: box-shadow 0.5s ease;
    }

    .tunnel {
        width: 220px;
        height: 220px;
        background-color: #ffffff;
        box-shadow: 0px 0px 5px 1px #52fefe, 0px 0px 5px 4px #52fefe inset;
        transition: box-shadow 0.5s ease;
    }

    // [ Coil animation ]--
    .coil-container {
        position: relative;
        width: 100%;
        height: 100%;
        animation: 10s infinite linear reactor-anim;
        transition: animation-duration 0.5s ease;
    }

    .coil {
        position: absolute;
        width: 30px;
        height: 20px;
        top: calc(50% - 110px);
        left: calc(50% - 15px);
        transform-origin: 15px 110px;
        background-color: #073c4b;
        box-shadow: 0px 0px 5px #52fefe inset;
    }

    @for $i from 1 through 8 {
        .coil-#{$i} {
            transform: rotate(#{($i - 1) * 45}deg);
        }
    }

    @keyframes reactor-anim {
        from { transform: rotate(0deg); }
        to { transform: rotate(360deg); }
    }

    // [ Arc element ]--
    .e7 {
        position: relative;
        z-index: 1;
        width: 160%;
        height: 160%;
        left: -32.5%;
        top: -32.5%;
        right: 0;
        bottom: 0;
        margin: auto;
        border: $size3 solid transparent;
        background: transparent;
        border-radius: 50%;
        transform: rotateZ(0deg);
        transition: box-shadow 3s ease, opacity 0.5s ease;
        text-align: center;
        opacity: 0.3;
    }

    .semi_arc_3 {
        content: "";
        position: absolute;
        width: 94%;
        height: 94%;
        left: 3%;
        top: 3%;
        border: 5px solid #02feff;
        border-radius: 50%;
        box-sizing: border-box;
        animation: rotate 4s linear infinite;
        text-align: center;
        overflow: hidden;
    }

    @keyframes rotate {
        0% { transform: rotateZ(0deg); }
        100% { transform: rotateZ(360deg); }
    }

    @keyframes rotate_anti {
        0% { transform: rotateZ(0deg); }
        100% { transform: rotateZ(-360deg); }
    }

    // [ Marks ]--
    .marks {
        li {
            width: 11px;
            height: 11px;
            background: $cshadow;
            position: absolute;
            margin-left: 117.5px;
            margin-top: 113.5px;
            animation: colour_ease2 3s infinite ease-in-out;
        }
    }

    @keyframes colour_ease2 {
        0% { background: $marks-color-1; }
        50% { background: $marks-color-2; }
        100% { background: $marks-color-1; }
    }

    @for $i from 1 through 60 {
        .marks li:nth-child(#{$i}) {
            transform: rotate(#{$i * 6}deg) translateY($arc-radius);
        }
    }

    // [ DISCONNECTED state ]--
    .reactor-container.disconnected {
        transform: scale(0.8);
        opacity: 0.4;
        filter: grayscale(0.8) brightness(0.6);

        .coil-container {
            animation-duration: 20s;
        }

        .state-label {
            opacity: 0.5;
        }
    }

    // [ IDLE state ]--
    .reactor-container.idle {
        transform: scale(0.9);
        opacity: 0.9;

        .coil-container {
            animation-duration: 10s;
        }

        .reactor-container-inner {
            box-shadow: 0px 0px 50px 15px $colour3, inset 0px 0px 50px 15px $colour3;
        }
    }

    // [ ACTIVE state (listening/processing) ]--
    .reactor-container.active {
        transform: scale(1);
        opacity: 1;

        .coil-container {
            animation-duration: 3s;
        }

        .reactor-container-inner {
            box-shadow: 0px 0px 70px 25px $colour3, inset 0px 0px 70px 25px $colour3;
        }

        .core-inner {
            box-shadow: 0px 0px 15px 10px #52fefe, 0px 0px 20px 15px #52fefe inset;
        }

        .core-outer {
            box-shadow: 0px 0px 5px 3px #52fefe, 0px 0px 15px 10px #52fefe inset;
        }

        .core-wrapper {
            box-shadow: 0px 0px 10px 8px #52fefe, 0px 0px 10px 5px #52fefe inset;
        }

        .tunnel {
            box-shadow: 0px 0px 10px 3px #52fefe, 0px 0px 10px 8px #52fefe inset;
        }

        .e7 {
            opacity: 0.6;
        }

        .e5_1 { animation: rotate 3s linear infinite; }
        .e5_2 { animation: rotate_anti 2s linear infinite; }
        .e5_3 { animation: rotate 2s linear infinite; }
        .e5_4 { animation: rotate_anti 2s linear infinite; }

        .marks li {
            animation: colour_ease2_active 1s infinite ease-in-out;
        }
    }

    @keyframes colour_ease2_active {
        0% { background: $marks-color-1; box-shadow: 0 0 5px $marks-color-1; }
        50% { background: $marks-color-2; box-shadow: none; }
        100% { background: $marks-color-1; box-shadow: 0 0 5px $marks-color-1; }
    }

    // [ Pulse animation for listening ]--
    @keyframes listening-pulse {
        0%, 100% { 
            transform: scale(1);
        }
        50% { 
            transform: scale(1.03);
        }
    }


    // [ State Label ]

    .state-label {
        margin-top: 1.5rem;
        display: flex;
        align-items: center;
        gap: 0.5rem;
    }

    .status-dot {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: #52fefe;
        box-shadow: 0 0 8px #52fefe;
    }

    .label-text {
        font-size: 0.9rem;
        color: rgba(82, 254, 254, 0.8);
        text-transform: uppercase;
        letter-spacing: 3px;
        font-weight: 400;
        font-family: "Roboto Condensed", sans-serif; // "Trebuchet MS", Arial, Helvetica, sans-serif
    }

    .reactor-container.active + .state-label {
        .status-dot {
            animation: dot-pulse 0.8s ease-in-out infinite;
        }
        .label-text {
            color: #52fefe;
        }
    }

    .reactor-container.disconnected + .state-label {
        .status-dot {
            background: #445555;
            box-shadow: none;
        }
        .label-text {
            color: #445555;
        }
    }

    @keyframes dot-pulse {
        0%, 100% { transform: scale(1); opacity: 1; }
        50% { transform: scale(1.5); opacity: 0.7; }
    }
</style>