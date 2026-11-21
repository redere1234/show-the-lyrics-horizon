# Show the Lyrics Horizon

Mod de Horizon para reproducir canciones propias dentro de Minecraft y mostrar sus letras sincronizadas al ritmo.

## Mod listo para instalar

El paquete compilado está en [`releases/show-the-lyrics-horizon-1.13.0.hmod`](releases/show-the-lyrics-horizon-1.13.0.hmod). Ese archivo es el que se puede subir a Horizon.

## Controles principales

- `J`: abrir el menú.
- `M`: reproducir.
- `N`: siguiente canción.
- `B`: canción anterior.
- `K`: mostrar u ocultar letras.
- `,` y `.`: ajustar el retraso de las letras.
- `O`: tomar una foto.

Las canciones y sus letras se cargan desde el paquete de recursos **Mi Música**.

## Contenido del repositorio

- `src/lib.rs`: código fuente del mod.
- `assets/`: fuentes, idiomas y recursos incluidos en el paquete compilado.
- `mod.wasm`: binario compilado incluido en el `.hmod`.
- `horizon-mod.toml`: metadatos de Horizon.
- `releases/`: paquete `.hmod` listo para cargar.

## Compilación

El paquete `.hmod` incluido corresponde a la versión `1.13.0` y conserva la compilación original del mod. Para recompilar desde el código fuente se necesita el SDK de Horizon y la herramienta `cargo-horizon-mod` configurados para el toolchain indicado en `horizon-mod.toml`.

## Licencia

MIT. Creado por Dinorap4009. No es un producto oficial de Mojang ni de Microsoft.
