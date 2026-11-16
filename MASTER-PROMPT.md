Tu reprends le travail sur Resilience, dans ce repo. Tu écris la majeure partie du code. Sois honnête : si une idée est mauvaise ou trop ambitieuse, dis-le. Si tu ne sais pas, dis-le ou cherche, n'invente rien.

## Le projet
- « Resilience » = le projet cyberdeck : un ordinateur portable autonome et offline-first.
- « Resilience OS » = le système qui tourne dessus (NixOS + Rust, uniquement en terminal). Ce n'est plus un OS généraliste.
- Décision V1 (2026-11-15) : un vieux portable x86 recyclé, remonté dans un boîtier imprimé en 3D. On reste en x86_64 uniquement, pas de portage ARM. La LoRa 868 MHz et la SDR arrivent en V2.
- CLAUDE.md §1 est la référence. Il l'emporte sur MASTER-PROMPT.md et sur les ADR 0001-0016, qui décrivent encore une distribution généraliste.

## Avant tout
1. Lance `git fetch origin`, puis `git status` et `git branch -a`.
2. Le remote local pointe peut-être encore sur l'ancien nom du repo : le bon est raph-dot-img/resilience-os.
3. Lis CLAUDE.md, ROADMAP.md, README.md, et AGENT_HANDOFF.md sur origin/develop.

## État connu
- Branches : develop (stable), feat/maps (Phase 5 cartes : slices 1-5 faites et relues, slice 6 viewer en WIP, et le commit doc 960de32), feat/maps-builder (slice 7 en WIP). La branche peut changer : vérifie toujours où tu es avant d'écrire.
- Toute modif de CLAUDE.md ou ROADMAP.md se fait sur feat/maps, sinon on aura des conflits.
- AGENT_HANDOFF.md sur develop est la boîte aux lettres avec mon coordinateur en ligne. Les demandes sont numérotées R1, R2… Chaque commit de réponse ne touche que ce fichier. R1 = rebuild de l'ISO depuis develop. Traite-la si elle n'a pas encore de réponse.
- Les infos hardware sont marquées « not recorded yet ». Ne suppose rien, on remplira quand on aura trouvé le portable.

## Priorités (dans l'ordre)
1. Builder l'ISO depuis develop (`nix build .#iso`) et la tester dans QEMU (UEFI + BIOS si possible), en simulant une machine modeste (≤2 GiB de RAM). Note tout ce qui casse.
2. Préparer le test sur vrai matériel : une checklist de boot et un script qui relève les infos hardware (CPU, UEFI/BIOS, écran, stockage, RAM, Wi-Fi/BT/GPS) pour remplir la doc.
3. Seulement ensuite : finir la slice 6 (viewer) et la faire tourner sur une vraie carte, puis la slice 7.
Si je m'éparpille, ramène-moi à la V1 : un truc qui marche vaut mieux qu'un plan parfait.

## Questions ouvertes (ne tranche pas sans moi)
- Phase 8 : l'installateur texte « pour n'importe quel PC » devient sans doute « comment le deck reçoit et met à jour son image ». Propose, ne décide pas.
- build_resilience.sh : est-ce qu'on le garde ?

## Règles
- Je te laisse en autonomie. Push régulièrement pour ne rien perdre.
- Des commits petits, avec des messages clairs (en anglais). Pas de secrets dans le repo.
- Radio : réception libre, mais émission seulement sur les bandes autorisées sans licence en France (LoRa 868 MHz). Rappelle-moi la loi si je dérape.
- Quand une décision importante est prise, propose une mise à jour de CYBERDECK-BRIEF.md.

Commence par l'étape « Avant tout », puis attends mon OK.
