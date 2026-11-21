# CSB Model Optimizer

Make your Counter-Strike 1.6 / Half-Life server's downloads smaller, without changing how anything looks or plays.

Point it at your `cstrike` (or fast-download) folder. It writes an optimized copy and shows what your players
download before and after:

- **Pack animation.** Player models are mostly animation (often 85-95% of the file), and studiomdl stores a lot of it
  twice. The tool stores repeated sequences and repeated bone channels once. The model keeps its name and still
  works on every client, stock or not.
- **Share animation.** Player models that carry the same animation (skin packs, recolours, the same rig used
  again) can share one `models/player/_anims/<hash>.mdl` file. This uses GoldSrc's own sequence groups, so stock
  clients and servers load it with no plugin or engine change.
- **gzip.** It writes a `.gz` next to every file for fast-download hosts that serve them.
- **Warnings.** It finds texture files that are never loaded, broken `T.mdl` files, models without textures, and
  identical files under different names.

Every changed model is checked frame by frame against the original before it is written. If a check fails, the
model is kept as it was. Your input folder is never modified.

| Typical result (player models) | Raw | With gzip |
|---|---|---|
| Zombie Plague model pack, 62 files | 69.2 → 49.7 MB | 44.1 → 33.7 MB |
| Skins server, 152 player files | 266.6 → 191.4 MB | 160.8 → 118.7 MB |
| Whole server download folder, 1,350 files | 771 MB | → 393 MB |

---

## Tutorial

### 1. Get the app

**Option A: download (no tools needed).** Open the [Releases](../../releases) page and download
`csb-model-optimizer-windows-x64.zip`. Unzip it anywhere and run `csb-model-optimizer.exe`.

- It needs Windows 10 or 11 with the Microsoft Edge WebView2 runtime, which both already include.
- The exe is not code-signed, so Windows SmartScreen may say "Windows protected your PC". Click
  **More info → Run anyway**, or build it yourself (option B).

**Option B: build from source.**

1. Install Rust from <https://rustup.rs> (default options; it also installs the Visual Studio C++ build tools if
   you agree when asked).
2. Open PowerShell in this folder and run:

   ```powershell
   cargo build --release
   ```

3. The app is `target\release\csb-model-optimizer.exe`, and the command-line version is `target\release\mdlopt.exe`.

### 2. Optimize your files

1. Start `csb-model-optimizer.exe`.
2. **Game or fast-download folder:** click **Browse** and pick the folder that contains `models`, `sound`, `maps`
   and so on. That is usually `...\cstrike`, or your fast-download folder.
3. **Output folder:** filled in for you (`<folder>_optimized`). It must not be inside the input folder.
4. Choose what to do. The defaults are right for most servers:
   - **Pack animation**: on.
   - **Share animation**: on. Turn it off if you can't add files to your server's download list (see step 3).
   - **gzip copies**: `gzip -9`. Choose `Zopfli` for about 5% smaller `.gz` files, at the cost of a much slower run.
   - **Only models/**: tick it to skip maps and sounds and get a quicker run.
5. Press **Optimize**. A few hundred MB takes under a minute.

### 3. Read the result

- **Before / after:** total download size, estimated download times, and a bar chart per kind of file.
- **Files:** every changed file with its old and new size (filter, sort, "changed only").
- **Shared animation:** the shared files that were made and which models use them, with buttons to copy the
  lines for step 4.
- **Warnings:** problems the tool found but did not fix, such as a never-loaded `xxxT.mdl` or two identical skins
  under different names. Fix them yourself to save even more.

### 4. Put the files on your server

1. Copy the **output folder's contents** over your game server's `cstrike` folder, so the server and clients
   have the same files.
2. Upload the same files, **including the `.gz` files**, to your fast-download (`sv_downloadurl`) host.
3. **Only if animation was shared:** every player who downloads one of those skins also needs its
   `models/player/_anims/<hash>.mdl`. The list is in `shared_anims.txt` in the output folder (and on the
   **Shared animation** tab). Add the files either way:
   - one line per file in `maps/<mapname>.res` for each map; or
   - in the AMX Mod X plugin that precaches your skins, next to `precache_model`:

     ```c
     precache_generic("models/player/_anims/3ea0b04babce5061.mdl");
     ```

4. Change map. Players with the old files keep them (they still work). New players download the smaller ones.

### Command line

```powershell
mdlopt run "C:\server\cstrike" "C:\server\cstrike_optimized" --report report.json
mdlopt run IN OUT --no-share --gzip zopfli --models-only
mdlopt verify original.mdl optimized.mdl "C:\server\cstrike_optimized"
```

---

## Good to know

- **Stock skins are left alone.** `gign`, `sas`, `arctic` and the other stock CS models are checked by
  `mp_consistency`, and every player already has them.
- **Only `models/player/` models are split into shared files.** Weapon and other models are only packed.
- **Memory.** A shared file is made only if no model ends up holding more animation in memory than it did before.
- **Running it twice is safe.** Already-optimized models are recognised and left as they are.
- **Not covered yet:** model files with an old studio version (not v10), and models that already use external
  sequence groups (`model01.mdl`). They are copied unchanged.

## How it works

GoldSrc stores each bone's position and rotation channels as run-length-encoded streams. Each bone's
`mstudioanim_t` points at its streams with 16-bit offsets, and each sequence's `animindex` points at its table.
studiomdl writes every sequence as its own table followed by its own streams. The tool:

1. lets identical sequences share one table (`animindex` is 32-bit, so any distance works);
2. writes each distinct stream once within a chunk of sequences, keeping every offset within 65,535 bytes;
3. drops all-zero streams (offset 0 means "no motion" to the engine) and repeated trailing values (the engine
   repeats the last value of a run anyway);
4. for sharing, writes the chunks into an `IDSQ` sequence-group file and points the slim model's sequences at it.

The check after each rewrite decodes every sequence, bone, channel and frame the same way the engine does, and
compares the values. It also compares header values, bones, sequence descriptions, mesh and texture data.

## Build and test

```powershell
cargo test -p csb-mdl
cargo build --release
```

- `core/` is the library and the `mdlopt` CLI.
- `app/` is the Tauri 2 window; `app/ui` is plain HTML, CSS and JS. To check the layout in a browser, put a
  `mock-report.json` (from `mdlopt --report`) in `app/ui`, run `python -m http.server -d app/ui`, and open
  `http://localhost:8000/?mock`.

## License

MIT, see [LICENSE](LICENSE).
