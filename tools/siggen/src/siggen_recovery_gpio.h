#ifndef SIGGEN_RECOVERY_GPIO_H
#define SIGGEN_RECOVERY_GPIO_H
#include "siggen_peer.h"

/* Caller has selected GPIO mode, holds SDA low, and serializes with IRQs. */
bool siggen_recovery_gpio_arm(siggen_peer_t *peer, uint32_t generation);
/* Only PB11's enable/flag are changed; GPIO0_B may have other pin owners. */
void siggen_recovery_gpio_disarm(void);
#endif
