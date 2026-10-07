# Vidéo de présentation de maquette — sources

Vidéo de 40 s, 1920×1080, 30 i/s, calée sur une pulsation à 120 BPM
(1 temps = 0,5 s = 15 images). Tout est du code : une page HTML animée de façon
déterministe, capturée image par image, plus une bande-son synthétisée.

## Fichiers

| Fichier | Rôle |
|---|---|
| `shared.js` | **La timeline.** Durées des scènes (en temps musicaux), liste des 7 fonctionnalités de la rafale, des 4 projections et du passage cartoon → spacefill de la molécule (texte, clé de config, couleurs), courbes d'animation des paramètres 3D. |
| `index.html` | Les scènes : mise en page, textes, styles, animations. |
| `render_seq.mjs` | Rend les séquences 3D avec le vrai moteur maquette (WebAssembly) dans `seq/`. |
| `seq/` | Séquences 3D rendues (WebP), une image par image vidéo. Générées par `render_seq.mjs`, non versionnées. |
| `music.py` | La musique, synthétisée avec numpy/scipy → `music.wav`, plus `beats.json` (temps des kicks et des impacts) qui pilote les pulsations, flashs et secousses de l'image. |
| `capture.py` | Capture les 1200 images de `index.html` dans `frames/` (Playwright) : chaque image moyenne 4 sous-images (flou de mouvement, obturateur à 180°), puis grain, vignette, fuites de lumière et aberration chromatique sur les drops. |
| `build.sh` | Enchaîne musique → capture → encodage → `maquette-40s.mp4`. |

## Prérequis

- Node 18+ : `npm install`
- Python 3 : `pip install numpy scipy playwright` puis `playwright install chromium`
- `ffmpeg`
- Police « Latin Modern Roman » (optionnelle, pour le texte de la page PDF ; sinon une serif par défaut est utilisée)

## Aperçu en direct, avec le son

```sh
python3 -m http.server 8765
```

Ouvrez `http://localhost:8765/index.html?play` et cliquez dans la page : la vidéo
se joue en temps réel avec la musique (recliquez pour relancer). Pour inspecter
une image précise : `index.html?f=240` (numéro d'image, 30 par seconde).

## Reconstruire la vidéo

```sh
./build.sh
```

Le script copie les wasm du dépôt dans `packages/maquette-js`, rend les séquences
3D si `seq/` est absent (~5 min), synthétise la musique, capture les images et
encode `maquette-40s.mp4`. Seules les sources sont versionnées : `seq/`,
`frames/`, `music.wav` et la vidéo sont régénérés.

## Modifier

- **Textes, couleurs, mise en page** : `index.html`. Les libellés de la rafale de
  fonctionnalités sont dans `CUTS` (`shared.js`).
- **Rythme / durée** : `T` et `S2` dans `shared.js`. Si vous changez `DUR`,
  reportez-le dans `capture.py` (`DUR`) et `music.py` (`DUR` et les repères en
  tête de fichier).
- **Musique** : `music.py` — trance / techno douce en la mineur (nappes
  supersaw, basse sur les contretemps, arpège) ; accords et repères calés sur la
  timeline de `shared.js` (repères en tête de fichier). Pour utiliser votre propre piste, remplacez `music.wav` par
  un morceau à 120 BPM dont le « drop » tombe à 4,0 s et supprimez l'appel à
  `music.py` dans `build.sh`.
- **Rendus 3D** (modèle, caméra, ombrage) : `render_seq.mjs`, puis régénérez les
  séquences (supprimez `seq/` et relancez `build.sh`, ou rendez une partie seule).
  Toute modification de la timeline qui déplace une scène 3D nécessite aussi de
  les régénérer, car il y a une image par image vidéo. Le script utilise le dépôt
  parent ; `MAQUETTE_REPO` permet d'en indiquer un autre :

```sh
node render_seq.mjs all      # lapin, rafale, OpenSCAD (~2 min)
node render_seq.mjs tokyo    # scène glTF (~5 min ; en parallèle : `tokyo 0 3`, `tokyo 1 3`, `tokyo 2 3`)
```

## Crédits

Logo, modèles 3D et moteur de rendu : projet [maquette](https://github.com/bernsteining/maquette)
(voir la section « Models Credits » de sa documentation pour les modèles).
Dragon : Stanford 3D Scanning Repository (150 000 points échantillonnés, `examples/data/dragon.ply`).
Protéine : GroEL–GroES (PDB 1AON), rendue avec [molfig](https://typst.app/universe/package/molfig).
Polices : Bricolage Grotesque, JetBrains Mono (licence OFL).
