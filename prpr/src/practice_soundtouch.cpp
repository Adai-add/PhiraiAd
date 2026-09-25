// Narrow exception-safe C ABI. Tempo changes while pitch and rate remain 1.
#include "SoundTouch.h"
#include <cmath>
#include <memory>
extern "C" {
void *phira_tempo_new(unsigned sr, double tempo) noexcept {
    try {
        if (!sr || !std::isfinite(tempo) || tempo < 0.049 || tempo > 10.001) return nullptr;
        std::unique_ptr<soundtouch::SoundTouch> s(new soundtouch::SoundTouch());
        s->setSampleRate(sr); s->setChannels(2); s->setTempo(tempo);
        s->setSetting(SETTING_USE_QUICKSEEK, 1);
        return s.release();
    } catch (...) { return nullptr; }
}
void phira_tempo_free(void *p) noexcept { delete static_cast<soundtouch::SoundTouch*>(p); }
int phira_tempo_put(void *p, const float *samples, unsigned frames) noexcept {
    try { static_cast<soundtouch::SoundTouch*>(p)->putSamples(samples, frames); return 0; } catch (...) { return -1; }
}
int phira_tempo_receive(void *p, float *samples, unsigned frames) noexcept {
    try { return static_cast<soundtouch::SoundTouch*>(p)->receiveSamples(samples, frames); } catch (...) { return -1; }
}
int phira_tempo_clear(void *p) noexcept {
    try { static_cast<soundtouch::SoundTouch*>(p)->clear(); return 0; } catch (...) { return -1; }
}
int phira_tempo_flush(void *p) noexcept {
    try { static_cast<soundtouch::SoundTouch*>(p)->flush(); return 0; } catch (...) { return -1; }
}
}
