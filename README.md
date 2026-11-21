<div align="center">
  <img src="icon.png" alt="Icono de Show the Lyrics Horizon" width="180" />

  # Show the Lyrics Horizon

  **Letras sincronizadas, música propia y estilo dentro de Minecraft.**

  [![Versión](https://img.shields.io/badge/Horizon-1.13.1-7c3aed?style=for-the-badge)](horizon-mod.toml)
  [![Último commit](https://img.shields.io/github/last-commit/redere1234/show-the-lyrics-horizon?style=for-the-badge)](https://github.com/redere1234/show-the-lyrics-horizon/commits/main)
  [![Licencia MIT](https://img.shields.io/badge/licencia-MIT-22c55e?style=for-the-badge)](LICENSE)

  <a href="releases/show-the-lyrics-horizon-1.13.1.hmod">Descargar mod 1.13.1</a>
  ·
  <a href="#complemento-necesario-mi-música">Instalar Mi Música</a>
  ·
  <a href="#controles">Controles</a>
</div>

## ¿Qué hace?

**Show the Lyrics Horizon** convierte tus sesiones de Minecraft en una experiencia musical: reproduce tus canciones desde el paquete de recursos **Mi Música** y muestra sus letras sincronizadas con el ritmo, tanto en pantalla como en el mundo.

## Complemento necesario: Mi Música

> **Importante:** si quieres escuchar música dentro del mod, necesitas instalar también [`MiMusica-1.13.0.zip`](MiMusica-1.13.0.zip). El `.hmod` instala el mod, pero el complemento **Mi Música** contiene las canciones, carpetas de recursos, herramientas y letras que el mod utiliza. Sin él, el mod puede abrirse, pero no tendrá canciones propias para reproducir.

### Cómo instalar Mi Música

1. Descarga el mod [`show-the-lyrics-horizon-1.13.1.hmod`](releases/show-the-lyrics-horizon-1.13.1.hmod) y el complemento [`MiMusica-1.13.0.zip`](MiMusica-1.13.0.zip).
2. Extrae `MiMusica-1.13.0.zip`. Dentro encontrarás una carpeta llamada `MiMusica`.
3. Copia esa carpeta completa dentro de la carpeta `resourcepacks` de Minecraft. También puedes abrirla desde **Opciones → Paquetes de recursos → Abrir carpeta**.
4. Mete tus canciones en `MiMusica/PON_TUS_CANCIONES_AQUI/`. Se admiten formatos como `mp3`, `m4a`, `wav`, `flac`, `ogg`, `opus`, `webm` y `mp4`.
5. Ejecuta `AGREGAR_CANCIONES.bat` para convertir y preparar las canciones. Esta herramienta también intenta encontrar letras sincronizadas y calcula el ritmo.
6. Si quieres revisar o corregir resultados, ejecuta `PREPARAR_CANCIONES.bat`.
7. Abre Minecraft, activa **Mi Musica** en **Opciones → Paquetes de recursos** y deja el mod activado en **Opciones → Mods**.
8. Si el paquete ya estaba activo, pulsa `F3 + T` para recargar los recursos.

Para obtener mejores resultados, usa nombres como `Artista - Título`. Si tienes un archivo de letra sincronizada `.lrc`, colócalo junto a la canción con el mismo nombre.

## Instalación del mod

1. Descarga e instala el archivo `.hmod` desde [`releases/show-the-lyrics-horizon-1.13.1.hmod`](releases/show-the-lyrics-horizon-1.13.1.hmod).
2. Abre Horizon y entra en **Opciones → Mods**.
3. Activa **Show the Lyrics Horizon** y reinicia el juego si Horizon lo solicita.
4. Instala y activa también el paquete de recursos **Mi Musica** siguiendo los pasos anteriores.

## Controles

| Tecla | Acción |
|:---:|---|
| `J` | Abrir el menú de música |
| `M` | Reproducir o detener |
| `N` | Siguiente canción |
| `B` | Canción anterior |
| `K` | Mostrar u ocultar las letras |
| `,` / `.` | Ajustar el retraso de la letra |
| `O` | Tomar una foto |

## Estructura del complemento Mi Música

```text
MiMusica/
├── PON_TUS_CANCIONES_AQUI/       # Coloca aquí tus canciones y archivos .lrc
├── assets/musica/canciones/      # Canciones preparadas para el juego
├── assets/musica/letras/         # Letras sincronizadas
├── assets/musica/sounds/         # Sonidos convertidos
├── AGREGAR_CANCIONES.bat         # Agrega y prepara canciones nuevas
├── PREPARAR_CANCIONES.bat        # Revisa letras, datos y sincronización
└── pack.mcmeta                   # Metadatos del paquete de recursos
```

## Contenido del repositorio

- `icon.png`: icono actual del mod, 512×512.
- `horizon-mod.toml`: metadatos y versión de Horizon.
- `src/lib.rs`: código fuente.
- `mod.wasm`: binario compilado.
- `assets/musica/`: fuentes, idiomas y recursos del mod.
- `MiMusica-1.13.0.zip`: complemento de recursos necesario para usar música.
- `releases/`: paquetes `.hmod` listos para cargar.

## Compilación desde el código fuente

La versión incluida fue compilada originalmente con el SDK de Horizon. Para recompilar necesitas tener instalado `cargo-horizon-mod`, el SDK compatible y el toolchain indicado en [`horizon-mod.toml`](horizon-mod.toml).

## Créditos y licencia

Creado por **Dinorap4009** y publicado bajo licencia **MIT**. No es un producto oficial de Mojang ni de Microsoft.

<div align="center">
  <sub>Haz que cada canción tenga su propio momento.</sub>
</div>
