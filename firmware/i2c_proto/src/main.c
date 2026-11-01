/*
 * i2c_proto (T1.8): hardware I2C master bring-up on HPM6300EVK.
 *
 * Peer: HPM6750EVKMini running the HPM SDK sample
 * samples/drivers/i2c/interrupt/slave (address 0x16; receives 128 bytes,
 * then sends them back). Wiring: EVK J28[13]/[15] (I2C0 SCL/SDA) to
 * EVKMini P1[13]/[15], GND to GND. Start the slave first.
 *
 * Results are read over JTAG (this board's console is not readable): g_result.
 */

#include "board.h"
#include "hpm_i2c_drv.h"

#define TEST_I2C     BOARD_APP_I2C_BASE
#define PEER_ADDR    0x16U    /* SDK slave sample address */
#define ABSENT_ADDR  0x17U    /* nothing on the bus answers here */
#define N_BYTES      128U     /* SDK slave sample transfer size */

volatile struct {
    uint32_t magic;        /* 0xC0FFEE18 = run finished */
    uint32_t write_stat;   /* hpm_stat_t, 0 = success */
    uint32_t read_stat;
    uint32_t mismatches;   /* bytes read back != bytes written */
    uint32_t absent_stat;  /* expect status_i2c_no_addr_hit */
} g_result;

static uint8_t tx[N_BYTES];
static uint8_t rx[N_BYTES];

int main(void)
{
    board_init();
    board_init_i2c(TEST_I2C);

    for (uint32_t i = 0; i < N_BYTES; i++) {
        tx[i] = (uint8_t)i;
    }
    g_result.write_stat = i2c_master_write(TEST_I2C, PEER_ADDR, tx, N_BYTES);
    board_delay_ms(100);   /* the SDK slave needs time to switch to transmit */
    g_result.read_stat = i2c_master_read(TEST_I2C, PEER_ADDR, rx, N_BYTES);
    for (uint32_t i = 0; i < N_BYTES; i++) {
        if (rx[i] != tx[i]) {
            g_result.mismatches++;
        }
    }
    g_result.absent_stat = i2c_master_write(TEST_I2C, ABSENT_ADDR, tx, 1);
    g_result.magic = 0xC0FFEE18;

    while (1) {
    }
    return 0;
}
