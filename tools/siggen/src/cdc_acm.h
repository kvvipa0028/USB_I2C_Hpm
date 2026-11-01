#ifndef SIG5301_CDC_ACM_H
#define SIG5301_CDC_ACM_H

#include <stdint.h>
#include <stdbool.h>
#include <string.h>

extern volatile bool cdc_dtr;

void cdc_acm_init(uint8_t busid, uint32_t reg_base);
int cdc_getchar(void);
void cdc_write(const char *buf, uint32_t len);

#endif
