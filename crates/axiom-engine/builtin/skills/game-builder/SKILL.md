---
name: game-builder
description: Comprehensive development protocol for creating, debugging, and enhancing fully playable HTML5 Canvas and browser games with robust game loops, responsive controls, audio, and state management.
---

# Interactive Game Builder Protocol

This skill guides Axiom Agent in building, debugging, and polishing fully playable, high-performance HTML5 canvas and web games (e.g. Snake, Tetris, Pong, Space Invaders, Platformers, Roguelikes).

---

## 1. Core Architecture Principles

1. **Rock-Solid DOM & Canvas Initialization**:
   - Always query Canvas and UI elements safely after the DOM is ready or at script bottom.
   - Verify 2D context (`const ctx = canvas.getContext('2d')`) is defined before any drawing.
   - Handle device pixel ratios (`window.devicePixelRatio`) to ensure crisp rendering on high-DPI displays.

2. **Deterministic Game State Machine**:
   - Explicitly manage game states: `READY`, `PLAYING`, `PAUSED`, `GAME_OVER`.
   - Start and Reset handlers must cleanly reset all variables: score, snake body/velocity, food positions, timer, and animation frame IDs.
   - Never leave loose event listeners or uncancelled `requestAnimationFrame` loops on game over or restart.

3. **Responsive Input Handling**:
   - Provide intuitive keyboard controls (Arrow keys + WASD), touch/swipe controls for mobile, and spacebar for pause/start.
   - Prevent default browser scrolling (`e.preventDefault()`) on game control keys during gameplay.
   - Implement direction buffering to prevent instant 180-degree self-collisions when multiple keys are pressed rapidly.

4. **Visual Polish & Humanized Aesthetics**:
   - Clean dark-mode palette, glowing accents, grid lines, rounded corners, and particle bursts on score/eat/death.
   - Prominent HUD: Score, High Score (persisted to `localStorage`), and speed/level indicators.
   - Overlay modal screens with clear "Press Space or Click to Play Again" actions.

5. **Immediate Action (No Hesitation)**:
   - When the user asks to fix or build a game, immediately inspect the code, identify bugs, and execute `file.write` or `file.replace` in the same turn. Never stop at listing bugs without writing the fix.
