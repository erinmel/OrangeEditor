# Textos del paquete de audio en español. Los IDs son las claves de los nodos con
# "/", "." y "_" escritos como "-".

org-orange-audio-category-audio = Audio

org-orange-audio-play-label = Reproducir sonido
org-orange-audio-play-keywords = sonido, música, altavoz, canción, reproducir
org-orange-audio-loop-label = Repetir sonido



org-orange-audio-audio-out-label = Salida de audio
org-orange-audio-audio-out-description = Reproduce la señal conectada en el dispositivo seleccionado mientras la salida está activada.
org-orange-audio-audio-out-keywords = salida, altavoz, escuchar, silenciar, activar
org-orange-audio-play-from-label = Reproducir sonido de carpeta
org-orange-audio-record-label = Grabar en archivo

org-orange-audio-audio-source-label = Fuente de audio
org-orange-audio-audio-source-description = Proporciona la señal de un dispositivo o archivo. Conéctala a un analizador, una salida o un grabador.
org-orange-audio-audio-source-keywords = entrada de audio, micrófono, aplicación, sistema, cable virtual, archivo
org-orange-audio-audio-format-label = Información de la señal
org-orange-audio-audio-format-description = Indica la frecuencia de muestreo y el número de canales de la señal.
org-orange-audio-live-level-label = Nivel
org-orange-audio-live-level-description = Mide en dBFS el nivel de una señal mientras cambia.
org-orange-audio-live-level-keywords = volumen, intensidad, medidor
org-orange-audio-multiband-spectrum-label = Divisor de frecuencias
org-orange-audio-multiband-spectrum-description = Dibuja bandas en el espectro en directo. Cada banda entrega su propio flujo de audio filtrado.
org-orange-audio-multiband-spectrum-keywords = fft, frecuencia, filtro de banda, espectro de audio, graves, multibanda, separar audio, disparador de banda
org-orange-audio-noise-level-label = Disparador por nivel de audio
org-orange-audio-noise-level-description = Mide el volumen general y se dispara cuando permanece por encima o por debajo de un umbral.
org-orange-audio-noise-level-keywords = nivel de ruido, volumen, rms, umbral, micrófono
org-orange-audio-noise-level-then = Disparado
org-orange-audio-noise-gate-label = Compuerta de ruido
org-orange-audio-noise-gate-description = Silencia el audio de bajo nivel y entrega una señal para reproducir o grabar.
org-orange-audio-noise-gate-keywords = micrófono, puerta de ruido, noise gate, silenciar, ruido de fondo, umbral, filtro de audio
port-gated-stream = señal procesada
port-condition-met = condición cumplida
port-channel-levels = niveles por canal
org-orange-audio-transient-label = Transitorio
org-orange-audio-transient-description = Se dispara con cada golpe en la banda de una señal, como un bombo.
org-orange-audio-transient-keywords = bombo, golpe, ritmo, tambor, pulso
org-orange-audio-source-finished-label = Fuente terminada
org-orange-audio-source-finished-description = Se dispara cuando termina el archivo de sonido o la fuente de audio HTTP conectada. Para una captura en directo, usa el Disparador de nivel de audio.
org-orange-audio-source-finished-keywords = fin, terminado, pista
org-orange-audio-source-control-label = Control de fuente
org-orange-audio-source-control-description = Reproduce, pausa, reanuda, detiene o cambia la posición del archivo de sonido conectado.
org-orange-audio-sound-control-label = Control de sonidos
org-orange-audio-sound-control-description = Detiene, pausa, reanuda, cambia la posición o ajusta el volumen de los sonidos de una salida.

port-rate = frecuencia
port-channels = canales
port-gain = ganancia
port-low-hz = Hz mínimo
port-high-hz = Hz máximo
port-level = nivel
port-folder = carpeta
port-pick = elegir
port-file = archivo
port-seconds = segundos
port-recorded = grabado
port-command = comando
port-format = formato
org-orange-audio-compressor-label = Compresor
org-orange-audio-compressor-description = Baja las partes fuertes de una señal para que su nivel sea parejo. Ratio indica cuánto, Ataque y Liberación con qué rapidez, y Compensación vuelve a subir el resultado.
org-orange-audio-compressor-keywords = compresión, dinámica, volumen, nivel, voz, efecto de audio
org-orange-audio-limiter-label = Limitador
org-orange-audio-limiter-description = Mantiene los picos de una señal por debajo del techo para que nunca sature.
org-orange-audio-limiter-keywords = saturación, recorte, pico, techo, proteger, volumen, efecto de audio
org-orange-audio-gain-label = Ganancia
org-orange-audio-gain-description = Sube o baja el volumen de una señal en decibelios.
org-orange-audio-gain-keywords = volumen, más fuerte, más bajo, amplificar, atenuar, decibelios, efecto de audio
org-orange-audio-filter-label = Filtro
org-orange-audio-filter-description = Deja pasar las frecuencias graves (paso bajo), agudas (paso alto) o medias (paso banda) de una señal alrededor del corte. El modo de rechazo atenúa una banda estrecha alrededor del corte.
org-orange-audio-filter-keywords = ecualizador, eq, paso bajo, paso alto, paso banda, graves, agudos, frecuencia, efecto de audio

org-orange-audio-loop-description = Repite el archivo de sonido hasta que Control de sonido envía Detener a la misma salida. El paso siguiente se ejecuta al detener el bucle.
