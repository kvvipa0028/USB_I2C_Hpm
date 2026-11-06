#ifndef TEST_MCHTMR_H
#define TEST_MCHTMR_H
#include <stdint.h>
extern uint64_t test_now;
#define HPM_MCHTMR 0
static inline uint64_t mchtmr_get_count(int timer) { (void)timer; return test_now; }
#endif
