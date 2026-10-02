/*
 * vlcrs.h — public C ABI of libvlcrs (libvlcrs.so)
 *
 * libvlcrs is a lightweight Rust re-implementation of the libvlc playback core
 * for Android/arm64-v8a with first class spherical (VR) video support:
 *
 *   coverage   360° | 180° | planar (forced 2D)
 *   layout     mono | side-by-side | top-bottom
 *   + Auto     resolved from the container metadata (st3d/sv3d/proj/prhd/equi,
 *              Matroska StereoMode/Projection, Spherical Video V1 uuid XML)
 *   + eye      left/right selectable for stereo layouts
 *
 * A player instance is created from the Kotlin/Java side
 * (`org.videolan.libvlcrs.NativeBridge.nativeCreate`) because attaching the
 * output surface and delivering events need Java objects.  Every function below
 * then drives that instance through its opaque 64 bit handle, which makes the
 * engine usable from C/C++ or from another FFI layer (e.g. a Flutter plugin).
 *
 * All functions are safe to call with an unknown handle: they return
 * VLCRS_E_BAD_HANDLE (or 0/-1 for the getters) instead of crashing, and no call
 * can unwind across the ABI boundary.
 *
 * Thread safety: all functions may be called from any thread.  View updates
 * (drag/zoom/gyro/recenter/mode/eye) take effect on the next rendered frame and
 * never interrupt playback, so the current position is preserved.
 */

#ifndef VLCRS_H
#define VLCRS_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------- status -- */

enum {
    VLCRS_OK = 0,
    VLCRS_E_BAD_HANDLE = -1,
    VLCRS_E_BAD_ARGUMENT = -2,
    VLCRS_E_BAD_STATE = -3,
    VLCRS_E_UNSUPPORTED = -4,
    VLCRS_E_FAILED = -5
};

/* ----------------------------------------------------------------- state -- */

enum {
    VLCRS_STATE_IDLE = 0,
    VLCRS_STATE_OPENING = 1,
    VLCRS_STATE_PREPARED = 2,
    VLCRS_STATE_BUFFERING = 3,
    VLCRS_STATE_PLAYING = 4,
    VLCRS_STATE_PAUSED = 5,
    VLCRS_STATE_STOPPED = 6,
    VLCRS_STATE_END_REACHED = 7,
    VLCRS_STATE_ERROR = 8
};

/* ------------------------------------------------------ projection modes -- */

enum {
    VLCRS_MODE_AUTO = 0,     /* resolve from container metadata          */
    VLCRS_MODE_PLANAR = 1,   /* forced flat 2D                           */
    VLCRS_MODE_360_MONO = 2, /* 360° equirectangular, single eye         */
    VLCRS_MODE_360_SBS = 3,  /* 360° equirectangular, side by side       */
    VLCRS_MODE_360_TB = 4,   /* 360° equirectangular, top bottom         */
    VLCRS_MODE_180_MONO = 5, /* 180° hemisphere, single eye              */
    VLCRS_MODE_180_SBS = 6,  /* 180° hemisphere, side by side            */
    VLCRS_MODE_180_TB = 7    /* 180° hemisphere, top bottom              */
};

enum {
    VLCRS_EYE_LEFT = 0,
    VLCRS_EYE_RIGHT = 1
};

/* ------------------------------------------------------------ HUD fields -- */
/* Indices of the float array filled by vlcrs_get_view_info().               */

enum {
    VLCRS_HUD_YAW = 0,            /* degrees, positive = right              */
    VLCRS_HUD_PITCH = 1,          /* degrees, positive = up                 */
    VLCRS_HUD_ROLL = 2,           /* degrees                                */
    VLCRS_HUD_FOV_Y = 3,          /* vertical field of view, degrees        */
    VLCRS_HUD_FOV_X = 4,          /* horizontal field of view, degrees      */
    VLCRS_HUD_MODE = 5,           /* VLCRS_MODE_*                           */
    VLCRS_HUD_COVERAGE = 6,       /* 0 planar, 1 360, 2 180                 */
    VLCRS_HUD_LAYOUT = 7,         /* 0 mono, 1 SBS, 2 TB                    */
    VLCRS_HUD_EYE = 8,            /* 0 left, 1 right                        */
    VLCRS_HUD_SOURCE = 9,         /* 0 forced, 1 metadata, 2 fallback       */
    VLCRS_HUD_AT_YAW_LIMIT = 10,  /* 1 when the yaw boundary is reached     */
    VLCRS_HUD_AT_PITCH_LIMIT = 11,
    VLCRS_HUD_CONVERGING = 12,    /* 1 while the boundary is converging     */
    VLCRS_HUD_RESISTANCE = 13,    /* how hard the user pushes past the knee */
    VLCRS_HUD_GYRO = 14,
    VLCRS_HUD_GYRO_READY = 15,
    VLCRS_HUD_ZOOM = 16,
    VLCRS_HUD_POSITION_S = 17,
    VLCRS_HUD_DURATION_S = 18,
    VLCRS_HUD_FPS = 19,
    VLCRS_HUD_FLOAT_COUNT = 20
};

/* -------------------------------------------------------------- versions -- */

/* Library version, e.g. "1.0.0". The returned pointer is owned by libvlcrs. */
const char *vlcrs_version(void);
/* Human readable state name. */
const char *vlcrs_state_name(int32_t state);
/* Human readable projection mode name. */
const char *vlcrs_mode_name(int32_t mode);

/* ------------------------------------------------------------- lifecycle -- */

int32_t vlcrs_destroy(int64_t handle);

/* ----------------------------------------------------------------- media -- */

int32_t vlcrs_set_media_uri(int64_t handle, const char *uri);
int32_t vlcrs_set_media_fd(int64_t handle, int32_t fd, int64_t offset, int64_t length);
int32_t vlcrs_play(int64_t handle);
int32_t vlcrs_pause(int64_t handle);
int32_t vlcrs_resume(int64_t handle);
int32_t vlcrs_stop(int64_t handle);
int32_t vlcrs_seek_to(int64_t handle, int64_t position_ms);
int64_t vlcrs_get_time(int64_t handle);      /* ms, -1 for a bad handle */
int64_t vlcrs_get_length(int64_t handle);    /* ms, 0 when unknown      */
int32_t vlcrs_get_state(int64_t handle);     /* VLCRS_STATE_*           */
int32_t vlcrs_set_volume(int64_t handle, float volume); /* 0.0 … 1.0    */

/* -------------------------------------------------------------- VR view -- */

int32_t vlcrs_set_projection_mode(int64_t handle, int32_t mode);
int32_t vlcrs_get_projection_mode(int64_t handle);
int32_t vlcrs_set_eye(int64_t handle, int32_t eye);
int32_t vlcrs_get_eye(int64_t handle);
/* Invert the packing order advertised by the container (e.g. right eye first). */
int32_t vlcrs_set_swap_eyes(int64_t handle, int32_t swap);
/* Vertical field of view in degrees (clamped to 25…120). */
int32_t vlcrs_set_fov(int64_t handle, float fov_y);
/* Pinch zoom: factor > 1 zooms in. */
int32_t vlcrs_zoom(int64_t handle, float factor);
/* Single finger drag in screen pixels; the picture follows the finger. */
int32_t vlcrs_drag_pixels(int64_t handle, float dx, float dy);
/* Gyroscope look-around; display_rotation is 0/90/180/270. */
int32_t vlcrs_set_gyro(int64_t handle, int32_t enabled, int32_t display_rotation);
/* "视角摆正": drop the manual offsets and re-align the head tracker. */
int32_t vlcrs_recenter(int64_t handle);

/* ------------------------------------------------------------ read-outs -- */

/* Fill out[] with VLCRS_HUD_FLOAT_COUNT floats; returns the count written. */
int32_t vlcrs_get_view_info(int64_t handle, float *out, int32_t count);
/* Fill out[] with the media info, see VLCRS_MEDIA_INFO_COUNT below. */
int32_t vlcrs_get_media_info(int64_t handle, int32_t *out, int32_t count);
/* Fill out[] with the runtime counters, see VLCRS_STATS_COUNT below. */
int32_t vlcrs_get_stats(int64_t handle, int64_t *out, int32_t count);
/* Copy the formatted HUD text into buf; returns the bytes written, or the
 * required length when len == 0. */
int32_t vlcrs_hud_text(int64_t handle, char *buf, int32_t len);
/* Number of floats in the compact HUD representation. */
int32_t vlcrs_hud_float_count(void);

/* vlcrs_get_media_info layout */
enum {
    VLCRS_MEDIA_WIDTH = 0,
    VLCRS_MEDIA_HEIGHT = 1,
    VLCRS_MEDIA_ROTATION = 2,
    VLCRS_MEDIA_HAS_SPHERICAL = 3,
    VLCRS_MEDIA_COVERAGE = 4,     /* -1 unknown, 0 planar, 1 360, 2 180 */
    VLCRS_MEDIA_LAYOUT = 5,       /* -1 unknown, 0 mono, 1 SBS, 2 TB    */
    VLCRS_MEDIA_CONTAINER = 6,    /* 0 unknown, 1 mp4, 2 mkv, 3 other   */
    VLCRS_MEDIA_SAMPLE_RATE = 7,
    VLCRS_MEDIA_CHANNELS = 8,
    VLCRS_MEDIA_FPS_X100 = 9,
    VLCRS_MEDIA_INFO_COUNT = 10
};

/* vlcrs_get_stats layout */
enum {
    VLCRS_STAT_DECODED = 0,
    VLCRS_STAT_RENDERED = 1,
    VLCRS_STAT_DROPPED = 2,
    VLCRS_STAT_UNDERRUNS = 3,
    VLCRS_STAT_SEEKS = 4,
    VLCRS_STAT_FPS_X100 = 5,
    VLCRS_STAT_BUFFERED_MS = 6,
    VLCRS_STAT_EVENTS = 7,
    VLCRS_STATS_COUNT = 8
};

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* VLCRS_H */
