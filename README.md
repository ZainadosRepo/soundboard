# Soundboard

[![Release](https://img.shields.io/github/v/release/ZainadosRepo/soundboard)](https://github.com/ZainadosRepo/soundboard/releases/latest)
[![CI](https://github.com/ZainadosRepo/soundboard/actions/workflows/ci.yml/badge.svg)](https://github.com/ZainadosRepo/soundboard/actions/workflows/ci.yml)

Une soundboard pour le direct (théâtre, radio, stream, soirée) : chaque son est une tuile, rangée dans des sets affichés en grille ou en liste. Un clic lance ou arrête un son ; fondus, découpe, boucle, solo, volumes A/B, raccourcis clavier, contrôleurs MIDI et pilotage OSC sont réglables par tuile. Windows et macOS.

## Télécharger

Dernière version : **[Releases › Latest](https://github.com/ZainadosRepo/soundboard/releases/latest)**

| Système | Fichier |
| --- | --- |
| Windows 10 / 11 (64 bits) | `Soundboard_<version>_x64-setup.exe` |
| macOS (Intel et Apple Silicon) | `Soundboard_<version>_universal.dmg` |

### Windows

1. Lancer `Soundboard_<version>_x64-setup.exe`.
2. L'installeur n'est pas signé : si Windows SmartScreen s'affiche, cliquer sur **Informations complémentaires**, puis **Exécuter quand même**.
3. Suivre l'installeur ; Soundboard apparaît dans le menu Démarrer.

### macOS

1. Ouvrir le `.dmg` et glisser **Soundboard** dans **Applications**.
2. L'appli n'est pas signée : au premier lancement, macOS la bloque. Ouvrir **Réglages Système › Confidentialité et sécurité**, puis cliquer sur **Ouvrir quand même** en bas de la page.

### Mettre à jour

Au lancement, l'appli regarde sur GitHub si une version plus récente est publiée et propose de l'installer : **Installer** la télécharge, vérifie sa signature et redémarre l'appli ; **Plus tard** la reproposera au prochain lancement. Hors ligne, rien ne s'affiche. Les sets, les sons et les réglages sont conservés.

Jusqu'à la version 0.2.0 incluse, l'appli ne fait pas cette vérification : télécharger la nouvelle version et l'installer par-dessus l'ancienne.

## Utilisation

1. Déposer des fichiers audio (wav, aiff, mp3, m4a, flac, ogg) sur la fenêtre, ou cliquer sur **+ Sons** : chaque fichier devient une tuile. Le son est copié dans le set, qui ne dépend plus du fichier d'origine.
2. Cliquer sur une tuile pour la lancer, cliquer à nouveau pour l'arrêter.
3. Double-cliquer sur une tuile (ou **ⓘ**) pour ouvrir l'inspecteur et la régler.

| Action | Comment |
| --- | --- |
| Lancer / arrêter une tuile | Clic, ou sa touche |
| Mettre en pause une tuile | Maj + clic, ou Maj + sa touche |
| Arrêter une tuile en fondu | Ctrl + clic (⌘ + clic sur Mac) |
| Tout mettre en pause / reprendre | Espace |
| Fondu de sortie général | - |
| Tout arrêter | Échap |
| Jouer la tuile sélectionnée | Ctrl + Entrée |
| Volume général : basculer A/B, monter, baisser | Ctrl + Maj + D, Ctrl + ↑, Ctrl + ↓ |
| Nouveau set / nouveau smart set | Ctrl + Maj + N / Ctrl + Alt + N |
| Set précédent / suivant | Ctrl + Maj + [ / ] |
| Inspecteur | Ctrl + I |
| Éditeur de forme d'onde de la tuile sélectionnée | Ctrl + Maj + E |
| Apprendre la note MIDI de la tuile sélectionnée | Ctrl + L |
| Réinitialiser la liste | Ctrl + R |
| Réglages | Ctrl + , |
| Réordonner les tuiles | Glisser une tuile sur une autre |
| Options d'une tuile (boucle, solo, couleur…) | Clic droit sur la tuile |

Sur Mac, Ctrl est remplacé par ⌘.

### Tuiles

Dans l'inspecteur : titre, emoji, couleur, notes, touche du clavier (une lettre ou un chiffre), note MIDI, et :

- **Boucle** : le son repart au début ; son fondu d'entrée ne joue qu'au premier passage.
- **Solo** : lancer la tuile arrête tout ce qui joue (en fondu avec Ctrl + clic).
- **Maintenir pour jouer** : le son joue tant que la tuile, la touche ou le pad est enfoncé.
- **Volumes A et B** : deux niveaux par tuile ; on bascule de l'un à l'autre en douceur.
- **Découpe et fondus** : glisser les barres pour découper, les points jaunes pour les fondus. Le fichier son n'est jamais modifié.

L'éditeur de forme d'onde (**Ouvrir l'éditeur complet…**) ajoute le zoom (Ctrl + molette), la sélection à la souris, l'écoute de la sélection (Ctrl + Entrée), l'avance et le recul de 5 s (← →, Maj pour 1 s), Recadrer, Fondu d'entrée et de sortie sur la sélection, Revenir à l'original, et l'annulation (Ctrl + Z, Ctrl + Maj + Z).

### Sets

- **Grille** ou **liste** (boutons ▦ ☰). En liste, **Options** règle ce qui se passe à la fin d'un son : s'arrêter, sélectionner le suivant (Ctrl + Entrée le lance) ou enchaîner sur le suivant avec un fondu enchaîné. Les sons déjà joués sont cochés ; Ctrl + R remet la liste à zéro.
- **Smart set** : montre les tuiles des autres sets qui suivent des règles (favori, couleur, titre, boucle…).
- **Exporter / importer** (menu **⋯**) : un set et ses sons tiennent dans un seul fichier `.soundset`, à ouvrir sur un autre ordinateur (double-clic ou glisser sur la fenêtre).

### MIDI, OSC et liens

- **MIDI** : tous les contrôleurs branchés sont écoutés. Une note lance la tuile qui l'a apprise, la vélocité règle son volume. Dans **Réglages › MIDI**, choisir un appareil pour allumer ses pads pendant que leurs tuiles jouent.
- **OSC** : **Réglages › OSC** active la réception (port 8080 par défaut) pour piloter l'app depuis TouchOSC, un téléphone ou un autre logiciel, et l'envoi de l'état des tuiles. La liste des adresses est dans les réglages.
- **Liens `soundboard://`** : `soundboard://play?title=Nom`, `stop`, `fade`, `soundboard://set?title=Nom`, et toutes les adresses OSC (`soundboard://transport/stopAll`, `soundboard://master/volume?value=0.5`). Pratique depuis un Stream Deck ou un script.
- **Raccourci global** : **Réglages › Général** enregistre une combinaison (avec Ctrl, Alt ou ⌘) qui ramène la fenêtre au premier plan depuis n'importe quelle app.

### Où sont les fichiers

| | Windows | macOS |
| --- | --- | --- |
| Sets et sons | `%APPDATA%\io.github.zainadosrepo.soundboard\Sets\` | `~/Library/Application Support/io.github.zainadosrepo.soundboard/Sets/` |
| Réglages | `%APPDATA%\io.github.zainadosrepo.soundboard\settings.json` | `~/Library/Application Support/io.github.zainadosrepo.soundboard/settings.json` |

Un fichier de réglages illisible est mis de côté en `settings.json.bad` au lieu d'être écrasé ; un set illisible est signalé et ignoré sans masquer les autres.

## Développement

Prérequis : Rust stable, Node (tests de l'interface) et la CLI Tauri 2 :

```bash
cargo install tauri-cli --version "^2" --locked
```

Lancer l'appli :

```bash
cargo tauri dev
```

`SOUNDBOARD_LIBRARY_DIR=<dossier>` fait travailler l'appli sur une autre bibliothèque de sets, par exemple pour des essais.

| Dossier | Contenu |
| --- | --- |
| `crates/soundboard-core` | Ce qui se décide sans audio ni interface : sets, règles de lecture, fondus, MIDI, OSC, liens, formes d'onde, stockage |
| `src-tauri` | L'appli : moteur audio (décodage, mixage, sortie), MIDI, OSC, raccourcis, commandes de l'interface |
| `ui` | L'interface (HTML, CSS et modules JavaScript, sans étape de build) |

### Tests

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test "tests/ui/*.test.js"
```

`cargo test -p soundboard -- --ignored` joue en plus de vrais sons (volume coupé) sur la sortie audio de la machine.

### Publier une version

1. Monter la version dans `src-tauri/tauri.conf.json` et `src-tauri/Cargo.toml`.
2. Pousser un tag `v<version>` : `.github/workflows/release.yml` construit l'installeur Windows et l'image macOS dans une release en brouillon.
3. Relire la release sur GitHub et la publier.

Les mises à jour sont signées : la release a besoin des secrets `TAURI_SIGNING_PRIVATE_KEY` et `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` du dépôt, dont la clé publique est `plugins.updater.pubkey` dans `tauri.conf.json`. Les applis installées ne voient la version qu'une fois la release publiée (le brouillon est ignoré). Perdre la clé privée oblige à réinstaller à la main la version qui embarque la nouvelle clé publique.
