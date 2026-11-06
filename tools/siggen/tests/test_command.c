#include "../src/siggen_command.h"

#include <assert.h>

static void ok(const char *s, siggen_command_kind_t kind)
{
    siggen_command_t c;
    assert(siggen_command_parse(s, &c));
    assert(c.kind == kind);
}

static void bad(const char *s)
{
    siggen_command_t c;
    assert(!siggen_command_parse(s, &c));
}

int main(void)
{
    siggen_command_t c;
    siggen_line_assembler_t a;
    char long_line[161];

    ok("start", SIG_CMD_START); ok("release", SIG_CMD_RELEASE); ok("slave 0x08 1000000", SIG_CMD_SLAVE);
    assert(siggen_command_parse("table write 0x10 00aAff", &c) && c.data_len == 3U && c.data[1] == 0xaaU);
    assert(siggen_command_parse("hold both 5000", &c) && c.value[0] == 3U && c.value[1] == 5000U);
    ok("table read 255 1", SIG_CMD_TABLE_READ); ok("nack data 1", SIG_CMD_NACK_DATA);
    assert(siggen_command_parse("nack data 256", &c) && c.value[0] == 256U);
    {
        char hex32[65];
        for (size_t i = 0; i < 64U; ++i) hex32[i] = 'a';
        hex32[64] = '\0';
        assert(siggen_command_parse("table write 0 ", &c) == false);
        {
            char line[80] = "table write 0 ";
            size_t n = 14U;
            for (size_t i = 0; i < 64U; ++i) line[n++] = hex32[i];
            line[n] = '\0';
            assert(siggen_command_parse(line, &c) && c.data_len == 32U);
            line[n++] = 'a'; line[n] = '\0';
            assert(!siggen_command_parse(line, &c));
        }
    }
    bad("start x"); bad("slave 7 100000"); bad("stretch +1"); bad("stretch 200001"); bad("hold scl 0");
    bad("table read 255 2"); bad("table write 255 aa00bb"); bad("table write 0 abc"); bad("nack address 2"); bad("nack data 257");
    bad("start\x01"); bad("master 8 100000 garbage");

    siggen_line_assembler_init(&a);
    for (const char *p = "start"; *p; ++p) assert(siggen_line_assembler_feed(&a, (unsigned char)*p) == SIG_LINE_NONE);
    assert(siggen_line_assembler_feed(&a, '\r') == SIG_LINE_READY);
    assert(siggen_line_assembler_feed(&a, '\n') == SIG_LINE_NONE);
    assert(siggen_line_assembler_feed(&a, '\n') == SIG_LINE_READY);
    siggen_line_assembler_init(&a);
    assert(siggen_line_assembler_feed(&a, '\t') == SIG_LINE_NONE);
    assert(siggen_line_assembler_feed(&a, '\n') == SIG_LINE_READY);
    siggen_line_assembler_init(&a); assert(siggen_line_assembler_feed(&a, 0U) == SIG_LINE_NONE); assert(siggen_line_assembler_feed(&a, '\n') == SIG_LINE_REJECT);
    for (size_t i = 0; i < 160U; ++i) long_line[i] = 'x';
    long_line[160] = '\0';
    siggen_line_assembler_init(&a); for (size_t i = 0; i < 160U; ++i) assert(siggen_line_assembler_feed(&a, (unsigned char)long_line[i]) == SIG_LINE_NONE); assert(siggen_line_assembler_feed(&a, '\n') == SIG_LINE_REJECT);
    return 0;
}
