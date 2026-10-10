<!-- Based on: https://github.com/rembertdesigns/Iron-Man-Arc-Reactor-Pure-CSS -->
<!-- and https://codepen.io/FlyingEmu/pen/DZNqEj -->

<div id="arc-reactor" class="reactor-container">
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

<style lang="scss" global>
    // [ Variables ]--
    $arc-radius: 130px;
    $size3: 6px;
    $cshadow: rgba(2, 254, 255, 0.8);
    $marks-color-1: rgba(2, 254, 255, 1);
    $marks-color-2: rgba(2, 254, 255, 0.3);
    $colour1: rgba(2, 255, 255, 0.15);
    $colour3: rgba(2, 255, 255, 0.3);

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
    }

    .core-outer {
        width: 120px;
        height: 120px;
        border: 1px solid #52fefe;
        background-color: #ffffff;
        box-shadow: 0px 0px 2px 1px #52fefe, 0px 0px 10px 5px #52fefe inset;
    }

    .core-wrapper {
        width: 180px;
        height: 180px;
        background-color: #073c4b;
        box-shadow: 0px 0px 5px 4px #52fefe, 0px 0px 6px 2px #52fefe inset;
    }

    .tunnel {
        width: 220px;
        height: 220px;
        background-color: #ffffff;
        box-shadow: 0px 0px 5px 1px #52fefe, 0px 0px 5px 4px #52fefe inset;
    }

    // [ Coil animation ]--
    .coil-container {
        position: relative;
        width: 100%;
        height: 100%;
        animation: 10s infinite linear reactor-anim;
        transition: animation 1s;
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
        transition: box-shadow 3s ease;
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

    // generate mark rotations
    @for $i from 1 through 60 {
        .marks li:nth-child(#{$i}) {
            transform: rotate(#{$i * 6}deg) translateY($arc-radius);
        }
    }

    // [ Active state ]--
    .reactor-container.active {
        scale: 1.1;
        opacity: 1;

        .coil-container {
            animation: 3s infinite linear reactor-anim;
        }

        .reactor-container-inner {
            box-shadow: 0px 0px 50px 15px $colour3, inset 0px 0px 50px 15px $colour3;
        }

        .e5_1 { animation: rotate 3s linear infinite; }
        .e5_2 { animation: rotate_anti 2s linear infinite; }
        .e5_3 { animation: rotate 2s linear infinite; }
        .e5_4 { animation: rotate_anti 2s linear infinite; }
    }
</style>
