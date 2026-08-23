// Minimal streaming Ogg Vorbis decode API backed by stb_vorbis (public domain).
#ifndef DD_VORBIS_H
#define DD_VORBIS_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct dd_vorbis_file dd_vorbis_file;

/// Opens an Ogg Vorbis file. Returns NULL if the file is not readable Vorbis.
/// On success `channels`, `sample_rate` and `duration_seconds` are filled in.
dd_vorbis_file *dd_vorbis_open(const char *path,
                               int *channels,
                               int *sample_rate,
                               double *duration_seconds);

/// Reads up to `frames` interleaved float frames into `buffer`
/// (which must hold frames * channels floats). Returns frames actually read,
/// 0 at end of stream.
int dd_vorbis_read(dd_vorbis_file *file, float *buffer, int frames);

void dd_vorbis_close(dd_vorbis_file *file);

#ifdef __cplusplus
}
#endif

#endif
