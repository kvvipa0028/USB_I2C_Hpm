#include "wavegen.h"

#include <assert.h>
#include <string.h>

void wg_init(wavegen_t *w, i2cdec_edge_t *buf, size_t cap, uint64_t t0)
{
    memset(w, 0, sizeof(*w));
    w->edges = buf;
    w->cap = cap;
    w->t = t0;
    w->lvl[I2CDEC_SCL] = 1;
    w->lvl[I2CDEC_SDA] = 1;
    w->tsetup = 25;
    w->tlow = 50;
    w->thigh = 50;
}

void wg_set_timing(wavegen_t *w, uint32_t tsetup, uint32_t tlow, uint32_t thigh)
{
    w->tsetup = tsetup;
    w->tlow = tlow;
    w->thigh = thigh;
}

void wg_wait(wavegen_t *w, uint64_t ticks)
{
    w->t += ticks;
}

void wg_edge(wavegen_t *w, uint8_t line, uint8_t level)
{
    level &= 1u;
    if (w->lvl[line] == level) {
        return;
    }
    assert(w->n < w->cap);
    w->edges[w->n].t = w->t;
    w->edges[w->n].line = line;
    w->edges[w->n].level = level;
    w->n++;
    w->lvl[line] = level;
}

uint64_t wg_start(wavegen_t *w)
{
    uint64_t t;
    wg_edge(w, I2CDEC_SDA, 1);
    wg_edge(w, I2CDEC_SCL, 1);
    wg_wait(w, w->thigh);
    wg_edge(w, I2CDEC_SDA, 0); /* START condition */
    t = w->t;
    wg_wait(w, w->thigh);
    wg_edge(w, I2CDEC_SCL, 0);
    wg_wait(w, w->tlow);
    return t;
}

uint64_t wg_restart(wavegen_t *w)
{
    uint64_t t;
    wg_edge(w, I2CDEC_SDA, 1);
    wg_wait(w, w->tsetup);
    wg_edge(w, I2CDEC_SCL, 1);
    wg_wait(w, w->thigh);
    wg_edge(w, I2CDEC_SDA, 0); /* repeated START condition */
    t = w->t;
    wg_wait(w, w->thigh);
    wg_edge(w, I2CDEC_SCL, 0);
    wg_wait(w, w->tlow);
    return t;
}

uint64_t wg_stop(wavegen_t *w)
{
    uint64_t t;
    wg_edge(w, I2CDEC_SDA, 0);
    wg_wait(w, w->tsetup);
    wg_edge(w, I2CDEC_SCL, 1);
    wg_wait(w, w->thigh);
    wg_edge(w, I2CDEC_SDA, 1); /* STOP condition */
    t = w->t;
    wg_wait(w, w->thigh);
    return t;
}

uint64_t wg_bit(wavegen_t *w, uint8_t bit)
{
    uint64_t t;
    wg_edge(w, I2CDEC_SDA, bit & 1u);
    wg_wait(w, w->tsetup);
    wg_edge(w, I2CDEC_SCL, 1);
    t = w->t;
    wg_wait(w, w->thigh);
    wg_edge(w, I2CDEC_SCL, 0);
    wg_wait(w, w->tlow);
    return t;
}

uint64_t wg_byte(wavegen_t *w, uint8_t value, uint8_t ack)
{
    uint64_t t_msb = 0;
    int i;
    for (i = 7; i >= 0; i--) {
        uint64_t t = wg_bit(w, (value >> i) & 1u);
        if (i == 7) {
            t_msb = t;
        }
    }
    /* ACK slot */
    wg_edge(w, I2CDEC_SDA, ack ? 0 : 1);
    wg_wait(w, w->tsetup);
    wg_edge(w, I2CDEC_SCL, 1);
    wg_wait(w, w->thigh);
    wg_edge(w, I2CDEC_SCL, 0);
    wg_wait(w, w->tlow);
    return t_msb;
}

void wg_insert_glitch(i2cdec_edge_t *edges, size_t *n, size_t cap,
                      uint8_t line, uint64_t t, uint64_t width)
{
    i2cdec_edge_t e[2];
    size_t pos = 0, tail;
    assert(*n + 2 <= cap);
    while (pos < *n && edges[pos].t <= t) {
        pos++;
    }
    /* the glitch toggles away from this line's level just before t */
    {
        size_t k = pos;
        uint8_t lvl = 1;
        while (k-- > 0) {
            if (edges[k].line == line) {
                lvl = edges[k].level;
                break;
            }
        }
        e[0].t = t;
        e[0].line = line;
        e[0].level = lvl ^ 1u;
    }
    e[1].t = t + width;
    e[1].line = line;
    e[1].level = e[0].level ^ 1u;
    tail = *n - pos;
    memmove(&edges[pos + 2], &edges[pos], tail * sizeof(edges[0]));
    edges[pos] = e[0];
    edges[pos + 1] = e[1];
    *n += 2;
}
