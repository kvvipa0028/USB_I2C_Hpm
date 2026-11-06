#include "i2c_port.h"
#include "i2c_board.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>
I2C_Type test_regs[2]; uint64_t test_now;
unsigned test_resets[2]; uint8_t test_fifo[2][4], test_count[2], test_lines[2]={3,3};
bool test_gpio[2], test_scl_low[2], test_sda_low[2];
const bridge_board_port_t bridge_board_ports[2]={{&test_regs[0],0},{&test_regs[1],1}};
void bridge_board_mux(uint8_t p,bool gpio) { test_gpio[p]=gpio; }
void bridge_board_drive(uint8_t p,bool scl,bool sda) { test_scl_low[p]=scl; test_sda_low[p]=sda; }
uint8_t bridge_board_lines(uint8_t p) { return (uint8_t)(test_lines[p]&~((test_scl_low[p]?1U:0U)|(test_sda_low[p]?2U:0U))); }
void bridge_i2c0_isr(void);
void bridge_i2c3_isr(void);
static void event(unsigned p,uint32_t flags)
{ test_regs[p].STATUS=flags; bridge_ports_poll(); }
static void complete(unsigned p,bool ack)
{
    test_regs[p].CTRL &= ~255U; test_count[p]=0;
    event(p,I2C_STATUS_CMPL_MASK|I2C_STATUS_LINESCL_MASK|(ack?I2C_STATUS_ADDRHIT_MASK|I2C_STATUS_ACK_MASK:0));
}
static void master_tests(void)
{
    uint8_t w[2]={0xAA,0xBB},r[2]={0}; bridge_result_t result;
    bridge_transfer_t t0={0,0x50,1,0,100000,w,r,false},t1={1,0x51,1,0,400000,w,r,false};
    assert(bridge_port_transfer_validate(&t0)==0 && bridge_port_transfer_validate(&t1)==0);
    bridge_port_transfer_start(&t0); bridge_port_transfer_start(&t1);
    assert(test_regs[0].CMD==1 && test_regs[1].CMD==1);
    assert(!bridge_port_transfer_result(0,&result) && !bridge_port_transfer_result(1,&result));
    /* CH0 remains SCL-low while CH1 completes successfully. */
    test_regs[0].STATUS=0; complete(1,true);
    assert(bridge_port_transfer_result(1,&result) && result.status==BRIDGE_OK);
    assert(!bridge_port_transfer_result(0,&result));
    test_now=25051; event(0,0);
    assert(bridge_port_transfer_result(0,&result) && result.status==BRIDGE_TIMEOUT);
    assert(bridge_port_transfer_result(1,&result) && result.status==BRIDGE_OK);
    t0.probe=true; t0.write_len=0;
    assert(bridge_port_transfer_validate(&t0)==0); bridge_port_transfer_start(&t0);
    assert((test_regs[0].CTRL&I2C_CTRL_PHASE_DATA_MASK)==0 && (test_regs[0].CTRL&I2C_CTRL_PHASE_STOP_MASK));
    complete(0,false); assert(bridge_port_transfer_result(0,&result) && result.status==BRIDGE_NACK);
    /* Failed write half of a repeated-start request needs an explicit STOP. */
    t0.probe=false; t0.write_len=1; t0.read_len=1;
    assert(bridge_port_transfer_validate(&t0)==0); bridge_port_transfer_start(&t0);
    assert(!(test_regs[0].CTRL&I2C_CTRL_PHASE_STOP_MASK));
    complete(0,false);
    assert(!bridge_port_transfer_result(0,&result) && test_regs[0].CTRL==I2C_CTRL_PHASE_STOP_MASK);
    event(0,I2C_STATUS_CMPL_MASK|I2C_STATUS_LINESCL_MASK);
    assert(bridge_port_transfer_result(0,&result) && result.status==BRIDGE_NACK);
    /* Successful repeated START reads actual FIFO data. */
    assert(bridge_port_transfer_validate(&t0)==0); bridge_port_transfer_start(&t0); complete(0,true);
    assert(test_regs[0].CTRL&I2C_CTRL_DIR_SET(1));
    test_fifo[0][0]=0xC7;test_count[0]=1;test_regs[0].CTRL&=~255U;
    event(0,I2C_STATUS_CMPL_MASK|I2C_STATUS_ADDRHIT_MASK|I2C_STATUS_LINESCL_MASK);
    assert(bridge_port_transfer_result(0,&result) && result.status==BRIDGE_OK && r[0]==0xC7);
    /* Hardware progress never extends the independent total deadline. */
    assert(bridge_port_transfer_validate(&t0)==0);bridge_port_transfer_start(&t0);
    for(unsigned i=0;i<5;++i) { test_now+=99999;event(0,I2C_STATUS_LINESCL_MASK|I2C_STATUS_BYTETRANS_MASK|I2C_STATUS_ADDRHIT_MASK); }
    assert(!bridge_port_transfer_result(0,&result));test_now+=5;event(0,I2C_STATUS_LINESCL_MASK|I2C_STATUS_BYTETRANS_MASK);
    assert(bridge_port_transfer_result(0,&result) && result.status==BRIDGE_TIMEOUT);
}
static void slave_tests(void)
{
    bridge_port_state_t s;bridge_port_state(0,&s);bridge_port_config_t cfg=s.config;cfg.role=BRIDGE_SLAVE;
    unsigned other=test_resets[1];
    assert(bridge_port_configure(0,&cfg,true,s.config_generation)==0 && test_resets[1]==other);
    bridge_port_state(0,&s);uint64_t cg=s.config_generation;
    assert(bridge_port_configure(0,&cfg,true,cg-1)==BRIDGE_CONFLICT && test_resets[1]==other);
    test_regs[0].STATUS=I2C_STATUS_BUSBUSY_MASK;
    assert(bridge_port_configure(0,&cfg,false,0)==BRIDGE_BUSY);
    test_regs[0].STATUS=I2C_STATUS_ADDRHIT_MASK;test_regs[0].CTRL=0;bridge_i2c0_isr();
    test_fifo[0][0]=255;test_count[0]=1;test_regs[0].STATUS=I2C_STATUS_BYTERECV_MASK;bridge_i2c0_isr();assert(test_regs[0].CMD==2);
    test_fifo[0][0]=0x71;test_fifo[0][1]=0x72;test_count[0]=2;test_regs[0].STATUS=I2C_STATUS_BYTERECV_MASK;bridge_i2c0_isr();
    test_regs[0].STATUS=I2C_STATUS_CMPL_MASK;bridge_i2c0_isr();
    uint8_t table[256];uint64_t gen;
    assert(bridge_port_table_read(0,0,256,table,&gen)==0 && table[255]==0x71 && table[0]==0x72 && gen==2);
    /* Short write: address, received offset/data, completion in one snapshot. */
    test_regs[0].CTRL=0;test_fifo[0][0]=20;test_fifo[0][1]=0xAB;test_count[0]=2;
    test_regs[0].STATUS=I2C_STATUS_ADDRHIT_MASK|I2C_STATUS_BYTERECV_MASK|I2C_STATUS_CMPL_MASK;
    bridge_i2c0_isr();bridge_port_state(0,&s);assert(s.activity==0);
    assert(bridge_port_table_read(0,20,1,table,&gen)==0 && table[0]==0xAB && gen==3);
    /* Offset write followed by repeated START read. CMPL gates the boundary.
     * TX prefetch must not advance current offset beyond bytes actually sent. */
    test_regs[0].CTRL=0;test_fifo[0][0]=254;test_count[0]=1;
    test_regs[0].STATUS=I2C_STATUS_ADDRHIT_MASK|I2C_STATUS_BYTERECV_MASK|I2C_STATUS_CMPL_MASK;
    bridge_i2c0_isr();
    test_regs[0].CTRL=I2C_CTRL_DIR_SET(1);test_regs[0].STATUS=I2C_STATUS_ADDRHIT_MASK;bridge_i2c0_isr();
    assert(test_count[0]==4 && test_fifo[0][1]==0x71 && test_fifo[0][2]==0x72);
    test_regs[0].CTRL=I2C_CTRL_DIR_SET(1)|2;test_regs[0].STATUS=I2C_STATUS_CMPL_MASK;bridge_i2c0_isr();
    assert(test_count[0]==0 && !(test_regs[0].INTEN&I2C_EVENT_FIFO_EMPTY));
    test_regs[0].CTRL=I2C_CTRL_DIR_SET(1);test_regs[0].STATUS=I2C_STATUS_ADDRHIT_MASK;bridge_i2c0_isr();
    assert(test_fifo[0][0]==0x72); /* two sent bytes from 254 -> current offset 0 */
    test_regs[0].CTRL=I2C_CTRL_DIR_SET(1)|1;test_regs[0].STATUS=I2C_STATUS_CMPL_MASK;bridge_i2c0_isr();
    /* READ -> WRITE starts a fresh offset phase with TX-empty disabled. */
    test_regs[0].CTRL=0;test_regs[0].STATUS=I2C_STATUS_ADDRHIT_MASK;bridge_i2c0_isr();
    assert(!(test_regs[0].INTEN&I2C_EVENT_FIFO_EMPTY));
    test_fifo[0][0]=30;test_fifo[0][1]=0xCD;test_count[0]=2;
    test_regs[0].STATUS=I2C_STATUS_BYTERECV_MASK|I2C_STATUS_CMPL_MASK;bridge_i2c0_isr();
    assert(bridge_port_table_read(0,30,1,table,&gen)==0 && table[0]==0xCD && gen==4);
    uint8_t patch[2]={0x55,0x66};assert(bridge_port_table_write(0,10,2,patch,true,gen,&gen)==0 && gen==5);
    assert(bridge_port_table_write(0,10,2,patch,true,2,&gen)==BRIDGE_CONFLICT);
    cfg.role=BRIDGE_MASTER;assert(bridge_port_configure(0,&cfg,false,0)==0);
    assert(bridge_port_table_read(0,0,256,table,&gen)==BRIDGE_WRONG_ROLE);
    cfg.role=BRIDGE_SLAVE;assert(bridge_port_configure(0,&cfg,false,0)==0);
    assert(bridge_port_table_read(0,0,256,table,&gen)==0 && table[10]==0x55 && table[255]==0x71 && gen==5);
    /* Abort/reset of USB work preserves slave role and table. */
    bridge_ports_abort();bridge_port_state(0,&s);assert(s.config.role==BRIDGE_SLAVE && s.table_generation==5);
    cfg.role=BRIDGE_MASTER;assert(bridge_port_configure(0,&cfg,false,0)==0);
}
static void recovery_tests(void)
{
    bridge_recovery_result_t r;
    assert(bridge_port_recover_start(0)==0 && test_gpio[0]);
    unsigned other=test_resets[1];
    for(unsigned i=0;i<200;++i){++test_now;bridge_ports_poll();}
    assert(bridge_port_recover_result(0,&r) && r.reason==0 && r.clocks==9 && !test_gpio[0]);
    assert(test_resets[1]==other && !test_scl_low[0] && !test_sda_low[0]);
    test_lines[0]=2;assert(bridge_port_recover_start(0)==0);test_now+=25000;bridge_ports_poll();
    assert(bridge_port_recover_result(0,&r) && r.reason==1 && !test_gpio[0]);
    test_lines[0]=3;assert(bridge_port_recover_start(0)==0);bridge_ports_abort();
    assert(!test_gpio[0] && !test_scl_low[0] && !test_sda_low[0]);
}
int main(void)
{
    bridge_ports_init();master_tests();slave_tests();recovery_tests();
    puts("PASS actual port state machine with register substitute: overlap, deadlines, probe, STOP, roles/table/ISR, isolation");
}
