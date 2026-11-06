#ifndef SIG5301_CDC_ACM_H
#define SIG5301_CDC_ACM_H

#include <stdint.h>
#include <stdbool.h>
#include <string.h>

extern volatile bool cdc_dtr;

void cdc_acm_init(uint8_t busid, uint32_t reg_base);
int cdc_getchar(void);
bool cdc_can_write(void);
bool cdc_take_session_lost(void);
void cdc_stats(uint32_t *rx_overflows, uint32_t *tx_rejected);
bool cdc_write(const char *buf, uint32_t len);

#endif
