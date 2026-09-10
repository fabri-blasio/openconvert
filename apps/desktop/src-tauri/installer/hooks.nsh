; ============================================================================
; NSIS hooks — the uninstaller keeps a promise the licence page makes.
;
; The generated page (installer/LICENSE-AND-PRIVACY.txt) says:
;
;     "Settings, history and receipts are written under your user profile the
;      first time you run the program... Uninstalling offers to remove them."
;
; Tauri's uninstaller does show a "Delete the application data" checkbox, and
; when it is ticked it removes %APPDATA%\<bundle-id> and %LOCALAPPDATA%\<bundle-id>
; — for this app, `dev.openconvert.desktop`.
;
; THE APP DOES NOT WRITE THERE. `openconvert-run::state::paths::state_dir` resolves
; to %LOCALAPPDATA%\OpenConvert on Windows, one per-user directory shared by the
; GUI and the CLI, which has no bundle identifier to be named after. So the
; checkbox was removing an empty path that never existed, the config, history,
; recipes, receipts and batch journal stayed on disk, and the sentence above was
; false — which is the specific failure this project's gates exist to prevent.
;
; This hook deletes the directory the app actually uses, under exactly the two
; conditions the built-in block uses: the user ticked the box, and this is not
; an in-place update.
;
; Kept in step by `xtask desktop`, which fails the build if the config stops
; referencing this file while the licence page still makes the promise.
; ============================================================================

!macro NSIS_HOOK_POSTUNINSTALL
  ; $DeleteAppDataCheckboxState and $UpdateMode are the uninstaller's own
  ; variables. This macro is inserted after both are set, so it reads the
  ; user's answer rather than guessing at it.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    ; `current` regardless of install mode: the state directory is per-user
    ; even when the program was installed for every user, so a per-machine
    ; uninstall must still clear the profile of whoever is running it.
    SetShellVarContext current

    ; A fixed, fully-qualified path and nothing derived from user input. An
    ; uninstaller that composes a delete path from a variable is one bad
    ; variable away from being a deletion primitive, which is the same rule
    ; `03 §13` applies to the app's own temp sweep.
    RMDir /r "$LOCALAPPDATA\OpenConvert"
  ${EndIf}
!macroend
