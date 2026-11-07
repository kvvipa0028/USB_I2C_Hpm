#ifndef SIGGEN_BOARD_H
#define SIGGEN_BOARD_H
#include "siggen_peer.h"
void siggen_board_init(siggen_peer_t *peer);
const siggen_hal_t *siggen_board_hal(void);
uint64_t siggen_board_now_us(void);
uint32_t siggen_board_lock(void);
void siggen_board_unlock(uint32_t irq);
#endif
