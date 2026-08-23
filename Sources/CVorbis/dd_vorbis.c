#include "dd_vorbis.h"

#include <stdlib.h>

#define STB_VORBIS_HEADER_ONLY
#include "stb_vorbis.c"

struct dd_vorbis_file {
   stb_vorbis *handle;
   int channels;
};

dd_vorbis_file *dd_vorbis_open(const char *path,
                               int *channels,
                               int *sample_rate,
                               double *duration_seconds)
{
   int error = 0;
   stb_vorbis *handle = stb_vorbis_open_filename(path, &error, NULL);
   if (handle == NULL) return NULL;

   stb_vorbis_info info = stb_vorbis_get_info(handle);
   if (info.channels <= 0 || info.sample_rate <= 0) {
      stb_vorbis_close(handle);
      return NULL;
   }

   dd_vorbis_file *file = (dd_vorbis_file *)malloc(sizeof(dd_vorbis_file));
   if (file == NULL) {
      stb_vorbis_close(handle);
      return NULL;
   }
   file->handle = handle;
   file->channels = info.channels;

   if (channels) *channels = info.channels;
   if (sample_rate) *sample_rate = (int)info.sample_rate;
   if (duration_seconds) *duration_seconds = (double)stb_vorbis_stream_length_in_samples(handle)
                                             / (double)info.sample_rate;
   return file;
}

int dd_vorbis_read(dd_vorbis_file *file, float *buffer, int frames)
{
   if (file == NULL || buffer == NULL || frames <= 0) return 0;
   int got = stb_vorbis_get_samples_float_interleaved(file->handle,
                                                      file->channels,
                                                      buffer,
                                                      frames * file->channels);
   return got < 0 ? 0 : got;
}

void dd_vorbis_close(dd_vorbis_file *file)
{
   if (file == NULL) return;
   if (file->handle) stb_vorbis_close(file->handle);
   free(file);
}
