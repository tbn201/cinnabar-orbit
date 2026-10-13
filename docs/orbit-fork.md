# OrBit Cinnabar fork

The `orbit` branch contains OrBit's host integration on top of official Cinnabar. The original `dev` branch is retained as the fork's upstream starting point. Existing license and attribution files apply to this fork.

OrBit upstream updates runs daily and can also be started manually. It reads the official repository's default branch, compares it with `orbit`, and opens one update PR directly from upstream. It never resets or force-pushes the custom branch. Subsequent upstream commits appear in that open PR.

The workflow dispatches OrBit upstream compatibility to check the proposed merge on Linux, Windows and macOS. This explicit dispatch runs even when GitHub's own token created the PR. The check uses read-only permissions; the proposal job can open PRs and dispatch checks but cannot write source branches. No personal access token is stored in the fork.

Review the compatibility run and the upstream CI results, resolve any merge conflicts, and merge the update PR into `orbit`. Failed or conflicting updates remain open. A merge needs another live client check before packaging a release.

The inherited Package workflow builds the custom branch for macOS and Windows using Cinnabar's original packaging tools. Run it manually with the `orbit` branch selected after validation. It includes `local-mods`. An OrBit Launcher `.exe` or `.dmg` and a customized Cinnabar build are separate artifacts; building the launcher does not build or install the host.

The integration adds grant-checked events for hotbar selection, sprint latching, automatic forward sprinting, click debounce, Dressing Room navigation and a bounded player-name roster. Player latency remains unavailable until a real host measurement is supplied. There are no new unrestricted tick-speed, attack-input or reach capabilities in this setup.

Hitbox integration is in development. A versioned `orbit.hitbox-policy` event requests independent outlines and targeting-box padding for players, mobs or both. Padding is bounded to 0–1 blocks per side and observation range to 4–64 blocks. The host requires controls, player-state, players, entities, interaction and render grants, renews the request each frame, and clears it outside active gameplay. This changes client targeting only, preserves block obstruction and attack admission, and does not alter physical collision or server hitboxes. Outlines display up to 16 nearby actors with eight boxes each. Packaging awaits a live render and gameplay pass.

Auto Sprint uses the versioned `orbit.auto-sprint` request and separate `orbit.auto-sprint-ready` capability. It requires controls and movement grants, is renewed each frame, and is limited to focused, captured gameplay in the current player session and dimension. The host uses its existing always-sprint movement path, so movement eligibility and server authority remain unchanged. Older hosts retain Toggle Sprint and Auto Hotbar support.
