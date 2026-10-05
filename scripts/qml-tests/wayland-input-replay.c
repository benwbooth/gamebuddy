// Test only: turn private KWin wheel packets into protocol-exact finger axes
// or pinch callbacks at Gamescope's real Wayland listener boundary. No desktop
// devices, Chromium input injection, or production event handlers are replaced.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef void (*fn)(void);
struct slot {
  void *proxy, *data;
  fn original[11], wrapped[11];
  void *surface;
  uint32_t time;
  int32_t delta;
};
static struct slot pointers[4], pinch;
static struct slot *find(void *p) {
  for (int i = 0; i < 4; i++)
    if (pointers[i].proxy == p)
      return &pointers[i];
  return NULL;
}
static void enter(void *d, void *p, uint32_t serial, void *s, int32_t x,
                  int32_t y) {
  struct slot *h = find(p);
  h->surface = s;
  ((void (*)(void *, void *, uint32_t, void *, int32_t,
             int32_t))h->original[0])(d, p, serial, s, x, y);
}
static void axis(void *d, void *p, uint32_t t, uint32_t a, int32_t v) {
  struct slot *h = find(p);
  h->time = t;
  h->delta += v;
  if (strcmp(getenv("GAMEBUDDY_TEST_POINTER_REPLAY"), "pinch"))
    ((void (*)(void *, void *, uint32_t, uint32_t, int32_t))h->original[4])(
        d, p, t, a, v);
}
static void source(void *d, void *p, uint32_t s) {
  struct slot *h = find(p);
  ((void (*)(void *, void *, uint32_t))h->original[6])(d, p, 1);
}
static void discrete(void *d, void *p, uint32_t a, int32_t v) {}
static void frame(void *d, void *p) {
  struct slot *h = find(p);
  if (!strcmp(getenv("GAMEBUDDY_TEST_POINTER_REPLAY"), "pinch") && h->delta &&
      pinch.proxy) {
    ((void (*)(void *, void *, uint32_t, uint32_t, void *,
               uint32_t))pinch.original[0])(pinch.data, pinch.proxy, 1, h->time,
                                            h->surface, 2);
    ((void (*)(void *, void *, uint32_t, int32_t, int32_t, int32_t,
               int32_t))pinch.original[1])(pinch.data, pinch.proxy, h->time, 0,
                                           0, h->delta > 0 ? 307 : 205, 0);
    ((void (*)(void *, void *, uint32_t, uint32_t, int32_t))pinch.original[2])(
        pinch.data, pinch.proxy, 2, h->time + 1, 0);
  }
  h->delta = 0;
  ((void (*)(void *, void *))h->original[5])(d, p);
}
int wl_proxy_add_listener(void *p, fn *implementation, void *data) {
  int (*real)(void *, fn *, void *) = dlsym(RTLD_NEXT, "wl_proxy_add_listener");
  const char *(*name)(void *) = dlsym(RTLD_NEXT, "wl_proxy_get_class");
  if (!getenv("GAMEBUDDY_TEST_POINTER_REPLAY"))
    return real(p, implementation, data);
  unsetenv("LD_PRELOAD"); // Do not interpose on Gamescope's child applications.
  const char *n = name(p);
  if (!strcmp(n, "zwp_pointer_gesture_pinch_v1")) {
    pinch.proxy = p;
    pinch.data = data;
    memcpy(pinch.original, implementation, 3 * sizeof(fn));
  }
  if (!strcmp(n, "wl_pointer")) {
    for (int i = 0; i < 4; i++)
      if (!pointers[i].proxy) {
        struct slot *h = &pointers[i];
        h->proxy = p;
        h->data = data;
        // Gamescope binds wl_seat v8, whose pointer listener ends at
        // axis_value120.
        memcpy(h->original, implementation, 10 * sizeof(fn));
        memcpy(h->wrapped, implementation, 10 * sizeof(fn));
        h->wrapped[0] = (fn)enter;
        h->wrapped[4] = (fn)axis;
        h->wrapped[5] = (fn)frame;
        h->wrapped[6] = (fn)source;
        h->wrapped[8] = (fn)discrete;
        h->wrapped[9] = (fn)discrete;
        fprintf(stderr, "GAMEBUDDY_POINTER_REPLAY mode=%s\n",
                getenv("GAMEBUDDY_TEST_POINTER_REPLAY"));
        return real(p, h->wrapped, data);
      }
  }
  return real(p, implementation, data);
}
