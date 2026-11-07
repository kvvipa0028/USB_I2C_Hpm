#include "bridge_protocol.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

static bridge_port_state_t state[2];
static bridge_result_t results[2];
static bridge_transfer_t transfers[2];
static bool done[2];
static uint8_t tables[2][256];
static unsigned starts, mutations;
static uint32_t recover_reason;
static uint16_t get16(const uint8_t *b) { return (uint16_t)(b[0] | ((uint16_t)b[1] << 8)); }
static uint32_t get32(const uint8_t *b) { return get16(b) | ((uint32_t)get16(b+2) << 16); }
static uint64_t get64(const uint8_t *b) { return get32(b) | ((uint64_t)get32(b+4) << 32); }
static void put16(uint8_t *b, uint16_t n) { b[0]=(uint8_t)n; b[1]=(uint8_t)(n>>8); }
static void put32(uint8_t *b, uint32_t n) { put16(b,(uint16_t)n); put16(b+2,(uint16_t)(n>>16)); }
static void put64(uint8_t *b, uint64_t n) { put32(b,(uint32_t)n); put32(b+4,(uint32_t)(n>>32)); }
void bridge_port_state(uint8_t port, bridge_port_state_t *out) { assert(port<2); *out=state[port]; }
uint8_t bridge_port_configure(uint8_t port, const bridge_port_config_t *c, bool cas, uint64_t expected)
{
    assert(port<2);
    if (c->role>1 || c->slave_address<8 || c->slave_address>0x77 || c->speed_hz<10000 || c->speed_hz>1000000 ||
        c->stretch_us<1000 || c->stretch_us>100000 || c->transaction_us<c->stretch_us || c->transaction_us>2000000) return BRIDGE_BAD_REQUEST;
    if (cas && expected!=state[port].config_generation) return BRIDGE_CONFLICT;
    if (state[port].activity==1) return BRIDGE_BUSY;
    state[port].config=*c; ++state[port].config_generation; ++mutations;
    return BRIDGE_OK;
}
uint8_t bridge_port_table_read(uint8_t port,uint16_t off,uint16_t n,uint8_t *out,uint64_t *gen)
{
    assert(port<2 && n && off+n<=256);
    if (state[port].config.role!=BRIDGE_SLAVE) return BRIDGE_WRONG_ROLE;
    memcpy(out,tables[port]+off,n); *gen=state[port].table_generation; return BRIDGE_OK;
}
uint8_t bridge_port_table_write(uint8_t port,uint16_t off,uint16_t n,const uint8_t *in,bool cas,uint64_t expected,uint64_t *gen)
{
    assert(port<2 && n && off+n<=256);
    if (state[port].config.role!=BRIDGE_SLAVE) return BRIDGE_WRONG_ROLE;
    if(cas && expected!=state[port].table_generation) return BRIDGE_CONFLICT;
    memcpy(tables[port]+off,in,n); *gen=++state[port].table_generation; ++mutations; return BRIDGE_OK;
}
uint8_t bridge_port_transfer_validate(const bridge_transfer_t *t)
{
    if(t->port>=2 || t->address>0x7F || t->write_len>256 || t->read_len>256 ||
        (!t->probe && !t->write_len && !t->read_len) || t->speed_hz<10000 || t->speed_hz>1000000) return BRIDGE_BAD_REQUEST;
    if(state[t->port].config.role!=BRIDGE_MASTER) return BRIDGE_WRONG_ROLE;
    return BRIDGE_OK;
}
void bridge_port_transfer_start(const bridge_transfer_t *t)
{
    assert(t->port<2); transfers[t->port]=*t; ++starts; done[t->port]=false;
    for(unsigned i=0;i<t->read_len;++i) t->read_data[i]=(uint8_t)(i+t->port);
}
bool bridge_port_transfer_result(uint8_t port,bridge_result_t *r)
{ assert(port<2); if(!done[port])return false; *r=results[port]; return true; }
uint8_t bridge_port_recover_start(uint8_t port)
{ assert(port<2); if(state[port].config.role!=BRIDGE_MASTER)return BRIDGE_WRONG_ROLE; ++mutations; return BRIDGE_OK; }
bool bridge_port_recover_result(uint8_t port,bridge_recovery_result_t *out)
{ assert(port<2); *out=(bridge_recovery_result_t){9,1,3,recover_reason}; return true; }
void bridge_ports_init(void) {}
void bridge_ports_poll(void) {}
void bridge_ports_abort(void) {}

static bridge_protocol_t p;
static uint8_t req[512], rsp[512];
static void reset(void)
{
    memset(state,0,sizeof(state)); memset(results,0,sizeof(results)); memset(tables,0,sizeof(tables));
    starts=0; mutations=0; recover_reason=0;
    for(unsigned i=0;i<2;++i) state[i].config=(bridge_port_config_t){BRIDGE_MASTER,0x50,100000,25000,500000};
}
static uint32_t request(uint8_t cmd,uint8_t addr,uint16_t w,uint16_t r,uint16_t mps)
{
    memset(req,0,sizeof(req)); memset(rsp,0xCC,sizeof(rsp));
    req[0]=cmd; req[1]=0xA5; req[2]=addr; put16(req+4,w); put16(req+6,r);
    uint32_t n=8U+w; return n+(n%mps==0);
}
static void dispatch(uint32_t n,uint16_t mps)
{
    bridge_protocol_start(&p,req,n,mps,rsp);
    assert(rsp[0]==req[0] && rsp[1]==0xA5 && rsp[3]==0);
}
static void basic_tests(void)
{
    reset(); uint32_t n=request(1,0,0,0,64); dispatch(n,64);
    assert(bridge_protocol_poll(&p) && rsp[2]==0 && rsp[8]==1);
    n=request(2,0,0,0,512); dispatch(n,512);
    assert(p.response_len==28 && rsp[8]==1 && rsp[9]==2 && rsp[10]==7);
    assert(get16(rsp+12)==256 && get16(rsp+14)==240 && get32(rsp+24)==1000000);
    n=request(3,0,4,48,512); req[8]=1; dispatch(n,512);
    assert(rsp[2]==0 && p.response_len==56 && get32(rsp+12)==0x9F && get32(rsp+52)==0);
    assert(get32(rsp+20)==2000 && get32(rsp+48)==1500000);
    n=request(0x20,0,0,0,512); dispatch(n,512); assert(rsp[2]==BRIDGE_BAD_REQUEST && starts==0);
    n=request(0x19,0,4,24,64); dispatch(n,64); assert(rsp[2]==BRIDGE_UNSUPPORTED && p.response_len==8);
    n=request(0x1A,0,12,24,64); dispatch(n,64); assert(rsp[2]==BRIDGE_UNSUPPORTED && mutations==0);
}
static void transfer_tests(void)
{
    reset(); uint32_t n=request(0x10,0x50,256,256,512);
    memset(req+8,0x65,256); dispatch(n,512);
    assert(starts==1 && transfers[0].port==0 && transfers[0].speed_hz==100000);
    assert(transfers[0].write_len==256 && !bridge_protocol_poll(&p));
    done[0]=true; assert(bridge_protocol_poll(&p) && p.response_len==264 && rsp[263]==255);
    n=request(0x11,0x51,9,1,64); req[8]=1; put32(req+12,400000); req[16]=0xA7; dispatch(n,64);
    assert(starts==2 && transfers[1].write_data[0]==0xA7 && transfers[1].speed_hz==400000);
    results[1]=(bridge_result_t){BRIDGE_NACK,2003}; done[1]=true;
    assert(bridge_protocol_poll(&p) && p.response_len==8 && rsp[2]==BRIDGE_NACK && get32(rsp+4)==2003);
    /* FS exact-MPS requests require one zero short-packet padding byte. */
    n=request(0x10,0x50,56,0,64); assert(n==65); dispatch(n,64); assert(starts==3);
    dispatch(n-1,64); assert(rsp[2]==BRIDGE_BAD_REQUEST && starts==3);
    req[64]=1; dispatch(n,64); assert(rsp[2]==BRIDGE_BAD_REQUEST && starts==3);
    n=request(0x11,0x50,8,0,512); put32(req+12,100000); dispatch(n,512);
    assert(rsp[2]==BRIDGE_BAD_REQUEST && starts==3);
    state[0].config.role=BRIDGE_SLAVE;
    n=request(0x10,0x50,1,0,512); dispatch(n,512); assert(rsp[2]==BRIDGE_BAD_REQUEST && starts==3);
}
static uint32_t batch(uint16_t w0,uint16_t r0,uint16_t w1,uint16_t r1)
{
    uint32_t wlen=22U+w0+w1, rlen=18U+r0+r1;
    assert(wlen<=UINT16_MAX && rlen<=UINT16_MAX);
    uint32_t n=request(0x12,0,(uint16_t)wlen,(uint16_t)rlen,512);
    req[8]=2; uint32_t pos=10;
    for(unsigned i=0;i<2;++i) {
        uint16_t w=i?w1:w0, r=i?r1:r0;
        req[pos]=(uint8_t)i; req[pos+1]=0x50; put16(req+pos+2,w); put16(req+pos+4,r); put32(req+pos+6,100000);
        memset(req+pos+10,(int)(0x70+i),w); pos+=10U+w;
    }
    return n;
}
static void batch_tests(void)
{
    reset(); uint32_t n=batch(240,240,240,240); assert(n==510); dispatch(n,512);
    assert(starts==2 && !bridge_protocol_poll(&p)); done[0]=true; assert(!bridge_protocol_poll(&p));
    done[1]=true; assert(bridge_protocol_poll(&p) && p.response_len==506 && rsp[8]==2);
    n=batch(1,38,1,1); dispatch(n,512); done[0]=true; done[1]=true; results[1]=(bridge_result_t){BRIDGE_TIMEOUT,3};
    assert(bridge_protocol_poll(&p) && p.response_len==65 && rsp[64]==0);
    assert(rsp[2]==0 && rsp[11]==0 && rsp[57]==BRIDGE_TIMEOUT && get32(rsp+60)==3);
    unsigned before=starts;
    n=batch(1,1,1,1); req[21]=2; dispatch(n,512); assert(starts==before && rsp[2]==BRIDGE_BAD_REQUEST);
    n=batch(1,1,1,1); req[21]=0; dispatch(n,512); assert(starts==before && rsp[2]==BRIDGE_BAD_REQUEST);
    n=batch(1,1,1,1); put16(req+6,19); dispatch(n,512); assert(starts==before && rsp[2]==BRIDGE_BAD_REQUEST);
}
static void control_tests(void)
{
    reset(); uint32_t n=request(0x14,0,32,40,64); uint8_t *b=req+8;
    b[0]=1; b[1]=1; b[2]=BRIDGE_SLAVE; b[3]=0x52; put32(b+4,400000); put32(b+8,1000); put32(b+12,10000);
    dispatch(n,64); assert(rsp[2]==0 && state[1].config.role==BRIDGE_SLAVE && mutations==1 && get64(rsp+32)==1);
    put32(b+16,1); put64(b+24,0); dispatch(n,64); assert(rsp[2]==BRIDGE_CONFLICT && mutations==1 && p.response_len==8);
    put64(b+24,1); state[1].activity=1; dispatch(n,64); assert(rsp[2]==BRIDGE_BUSY && mutations==1);
    state[1].activity=0;
    uint16_t lengths[]={1,40,255,256}, offsets[]={0,1,254,255};
    for(unsigned i=0;i<4;++i) for(unsigned j=0;j<4;++j) {
        uint16_t len=lengths[i], off=offsets[j]; unsigned previous=mutations;
        n=request(0x16,0,16+len,16,64); b=req+8; b[0]=1; put16(b+2,off); put16(b+4,len); memset(b+16,0xA9,len);
        dispatch(n,64);
        if((uint32_t)off+len>256) { assert(rsp[2]==BRIDGE_BAD_REQUEST && mutations==previous); continue; }
        assert(rsp[2]==0 && get16(rsp+10)==off && get16(rsp+12)==len && mutations==previous+1);
        uint64_t gen=get64(rsp+16);
        n=request(0x15,0,8,16+len,64); b=req+8; b[0]=1; put16(b+2,off); put16(b+4,len); dispatch(n,64);
        assert(rsp[2]==0 && get64(rsp+16)==gen);
        for(unsigned k=0;k<len;++k) assert(rsp[24+k]==0xA9);
        uint16_t unpadded=24+len;
        assert(p.response_len==unpadded+(unpadded%64==0));
    }
    n=request(0x16,0,17,16,64); b=req+8; b[0]=1; b[1]=1; put16(b+4,1); put64(b+8,0);
    unsigned before=mutations; dispatch(n,64); assert(rsp[2]==BRIDGE_CONFLICT && mutations==before);
    n=request(0x15,0,8,17,64); put16(req+12,1); dispatch(n,64); assert(rsp[2]==BRIDGE_WRONG_ROLE);
    n=request(0x17,0,4,4,64); req[9]=0x77; dispatch(n,64); assert(transfers[0].probe && !transfers[0].write_len);
    done[0]=true; results[0]=(bridge_result_t){BRIDGE_NACK,2003}; assert(bridge_protocol_poll(&p) && rsp[10]==0 && rsp[2]==0);
    n=request(0x17,0,4,4,64); req[9]=0x78; before=starts; dispatch(n,64); assert(rsp[2]==BRIDGE_BAD_REQUEST && starts==before);
    n=request(0x18,0,4,8,64); recover_reason=2; dispatch(n,64);
    assert(bridge_protocol_poll(&p) && rsp[2]==0 && get32(rsp+12)==2 && rsp[9]==9);
}
static void malformed_tests(void)
{
    reset();
    uint32_t seed=0xC001CAFE;
    for(unsigned test=0;test<20000;++test) {
        for(unsigned i=0;i<512;++i) { seed=seed*1664525U+1013904223U; req[i]=(uint8_t)(seed>>24); }
        uint32_t n=test%513;
        bridge_protocol_start(&p,req,n,(test&1)?64:512,rsp);
        assert(p.response_len<=507);
    }
    assert(starts==0 && mutations==0);
}
int main(void)
{
    basic_tests(); transfer_tests(); batch_tests(); control_tests(); malformed_tests();
    puts("PASS actual protocol parser: legacy, controls, preflight, padding, partial results, malformed frames");
}
