<div align="center">
  <img src="icon.png" alt="Icono de Show the Lyrics Horizon" width="180" />

  # Show the Lyrics Horizon

  **Letras sincronizadas, música propia y estilo dentro de Minecraft.**

  [![Versión](https://img.shields.io/badge/Horizon-1.13.1-7c3aed?style=for-the-badge)](horizon-mod.toml)
  [![Último commit](https://img.shields.io/github/last-commit/redere1234/show-the-lyrics-horizon?style=for-the-badge)](https://github.com/redere1234/show-the-lyrics-horizon/commits/main)
  [![Licencia MIT](https://img.shields.io/badge/licencia-MIT-22c55e?style=for-the-badge)](LICENSE)

  <a href="releases/show-the-lyrics-horizon-1.13.1.hmod">Descargar mod 1.13.1</a>
  ·
  <a href="#instalación-rápida">Instalación</a>
  ·
  <a href="#controles">Controles</a>
</div>

## ¿Qué hace?

**Show the Lyrics Horizon** convierte tus sesiones de Minecraft en una experiencia musical: reproduce tus canciones desde el paquete de recursos **Mi Música** y muestra sus letras sincronizadas con el ritmo, tanto en pantalla como en el mundo.

> **Consejo:** prepara cada canción con su archivo `.lrc` para disfrutar de una sincronización más precisa.

## Instalación rápida

1. Descarga [`show-the-lyrics-horizon-1.13.1.hmod`](releases/show-the-lyrics-horizon-1.13.1.hmod).
2. Abre Horizon y entra en **Opciones → Mods**.
3. Importa o instala el archivo `.hmod`.
4. Activa el mod y reinicia el juego si Horizon lo solicita.
5. Añade tus canciones y letras al paquete de recursos **Mi Música**.

## Controles

| Tecla | Acción |
|:---:|---|
| `J` | Abrir el menú de música |
| `M` | Reproducir o pausar |
| `N` | Siguiente canción |
| `B` | Canción anterior |
| `K` | Mostrar u ocultar las letras |
| `,` / `.` | Ajustar el retraso de la letra |
| `O` | Tomar una foto |

## Cómo organizar la música

El mod busca tus canciones y sus datos dentro del paquete de recursos **Mi Música**. Usa los archivos de información y letras sincronizadas para que el título, artista, ritmo y texto aparezcan correctamente durante la reproducción.

## Estructura del repositorio

```text
├── icon.png                         # Icono actual del mod, 512×512
├── horizon-mod.toml                 # Metadatos y versión de Horizon
├── src/lib.rs                       # Código fuente
├── mod.wasm                         # Binario compilado
├── assets/musica/                   # Fuentes, idiomas y recursos
└── releases/
    ├── show-the-lyrics-horizon-1.13.0.hmod
    └── show-the-lyrics-horizon-1.13.1.hmod
```

## Compilación desde el código fuente

La versión incluida fue compilada originalmente con el SDK de Horizon. Para recompilar necesitas tener instalado `cargo-horizon-mod`, el SDK compatible y el toolchain indicado en [`horizon-mod.toml`](horizon-mod.toml).

## Créditos y licencia

Creado por **Dinorap4009** y publicado bajo licencia **MIT**. No es un producto oficial de Mojang ni de Microsoft.

<div align="center">
  <sub>Haz que cada canción tenga su propio momento.</sub>
</div>
