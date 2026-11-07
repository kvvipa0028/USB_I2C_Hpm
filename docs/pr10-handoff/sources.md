# PR10 来源与证据范围

本页不上传会话日志、个人信息、凭据或设备工作目录。只列公开来源与已整理的证明范围。

## 源码版本

项目文档记载的候选为HPM SDK 1.13.0，完整SHA为 `ccc5ec467a047f3ea85cd2fa4565e91db7135a38`，内含CherryUSB 1.6.1。项目通过环境指定SDK，未锁定该SHA；部署版本和dirty状态未证实。

产品审查基线为 `24743da0a3e5a09cbaeccdeab9c8a2bd282203bc`，PR9基线为 `43fd7477cece581f015b3b5d3852c62e65738e25`。以下hash是文件内容SHA256，不是Git blob SHA。

| 文件与固定SHA链接 | 文件SHA256 | 证明范围 |
|---|---|---|
| [middleware/cherryusb/port/hpmicro/usb_dc_hpm.c](https://github.com/hpmicro/hpm_sdk/blob/ccc5ec467a047f3ea85cd2fa4565e91db7135a38/middleware/cherryusb/port/hpmicro/usb_dc_hpm.c#L310-L316) | `654b75301c95ae4a126ba2b2d933714e6e1feb2d9fdeb616f490bc29ca0ff6cb` | reset回调314、bus_reset315；394–406为SETUP循环 |
| [middleware/cherryusb/core/usbd_core.c](https://github.com/hpmicro/hpm_sdk/blob/ccc5ec467a047f3ea85cd2fa4565e91db7135a38/middleware/cherryusb/core/usbd_core.c#L1016-L1095) | `409b74d21a58335d67c19e01cab99447c05443d408b94aad72d3627e34793f56` | 1086为数据或状态EP0 start；1007/1010为reset中的EP0 open |
| [drivers/inc/hpm_mchtmr_drv.h](https://github.com/hpmicro/hpm_sdk/blob/ccc5ec467a047f3ea85cd2fa4565e91db7135a38/drivers/inc/hpm_mchtmr_drv.h#L29-L42) | `c1f8670ef0f70bfb33d780a24cf96bd4c52c9724a175d7e1f8221483b8161e2e` | 直接64位读写helper，无显式hi-lo-hi |
| [soc/HPM6300/HPM6360/toolchains/trap.c](https://github.com/hpmicro/hpm_sdk/blob/ccc5ec467a047f3ea85cd2fa4565e91db7135a38/soc/HPM6300/HPM6360/toolchains/trap.c#L150-L198) | `01732426c2efd5a6560d17369afacc73c10370cab0c2ff2286786878c78a005a` | 162提前开MIE受宏控制；197/198为软件CSR恢复 |
| [drivers/src/hpm_usb_drv.c](https://github.com/hpmicro/hpm_sdk/blob/ccc5ec467a047f3ea85cd2fa4565e91db7135a38/drivers/src/hpm_usb_drv.c#L94-L120) | `febb7d37552303c01bc18cfa5d5e8da14efdaeb67becb54b2aa4a0bde250ef66` | 115–119无界PRIME/FLUSH等待；close的283–295也有等待 |
| [components/usb/device/hpm_usb_device.c](https://github.com/hpmicro/hpm_sdk/blob/ccc5ec467a047f3ea85cd2fa4565e91db7135a38/components/usb/device/hpm_usb_device.c#L75-L91) | `f224a232512687333ed9e419f761cf1768ef91def1b32df203dc1b7a792b4e9f` | flush之后memset；233为open memset，279–286为有计数EP0自旋 |
| [firmware/usb_i2c/src/usb_i2c.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/24743da0a3e5a09cbaeccdeab9c8a2bd282203bc/firmware/usb_i2c/src/usb_i2c.c#L23-L60) | `62ef0575a28a6298519735e84136f4c72d7abc1b72863d665bb43ce794e3480b` | 基线USB flags和epoch写点 |
| [firmware/usb_i2c/src/i2c_port.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/24743da0a3e5a09cbaeccdeab9c8a2bd282203bc/firmware/usb_i2c/src/i2c_port.c#L279-L301) | `e2c7a55538b4f38d1ef60f6079bca19fba0ce8b910c1a3061b656636751406e7` | master_finish、NACK补STOP及配置路径 |
| [firmware/common/i2c_table.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/24743da0a3e5a09cbaeccdeab9c8a2bd282203bc/firmware/common/i2c_table.c#L1-L58) | `3f95813b39694af7c56649a1f1f94a424b07662eca85c77bcf88c7076df83206` | 整表、generation、CAS和RESYNC写点 |

源代码存在写点和循环，不直接证明硬件时限、DMA静止、所有回调有界或已部署版本。此前交接校验过76份必要SDK源文件；完整校验清单属于本地工程证据，本页未伪造它的公开URL。上表列出本次关键可复核文件。

## HPM6750官方手册

官方PDF：[HPM6700/6400 User Manual Rev2.6](https://www.hpmicro.com/Public/Uploads/uploadfile2/files/20260701/HPM67006400UMENV26.pdf)。

- PDF SHA256：`077dd70e2084a3f40735d2ffe584b1f69c2a22e6be7cae9108ce2d17e845e61f`。
- 本轮文本转换SHA256：`eecbaa4de459060672f177628d5f0686187c854473388fb000fb2ed8fe360cc7`。该文本是交接转换件，不是官方单独发布的txt，不提供虚构公开URL。PDF页章是主要定位，LF行号只辅助定位；工具版本不同可能改变转换结果。
- 该手册适用HPM6750，不适用HPM6360。HPM6360用户手册尚未取得，不能混用两芯片保证。

| 内容 | 页章及本轮LF行号 | 能证明 | 不能证明 |
|---|---|---|---|
| MCHTMR | 物理PDF163页，印刷118/1345，§5.2.1，13001–13009 | 计数每时钟递增，计数≥compare发中断，有效中断期间写compare会清中断 | 清后何时重置位、持续电平、分半写、MMIO延迟、6360语义 |
| trap进入与返回 | §4.5，12889起 | MIE、MPIE的硬件保存恢复 | 所有软件回调不重开MIE、挂起中断响应最大时间 |
| SUTW | USBCMD字段，86490–86501 | hazard可清除tripwire | hazard精确定义和SETUP竞争完整模型 |
| SETUPSTAT | 印刷1162页，§61.6.21，88670–88681 | 手册要求读取setup后写1清除 | 新SETUP与清除并发时的完整语义 |
| PRIME | 印刷1163页，§61.6.22，88688–88757 | 存在瞬时变化的描述 | prime最大完成时间 |
| FLUSH | 印刷1164页，§61.6.23，88761–88816 | 在途包continues until completion，flush成功后位清 | 未给足以确认内存DMA静止先后的保证，不是证明硬件一定不静止 |
| STAT | 印刷1165页，§61.6.24，88817–88888 | dTD退役重新prime时可能momentarily cleared | 单次或有限次读0就是稳定DMA静止 |
| SLOM | USBMODE，88589–88595 | 引用Control Endpoint Operation Model | 本轮文本未找到该模型正文，不等于官方模型不存在 |

## 勘误和证据缺口

reset准确行号为usb_dc_hpm.c的314回调、315底层bus_reset。先前把315改成314的勘误已撤回。trap.c的197、198是软件写，不是“运行期无CPU写者”。EP0无数据请求的状态start在core1086，不能只拦1132或1190。

一次辅助摘录曾用splitlines拆分PDF换页符，造成行号偏移，后来已按LF行号及物理PDF163页独立核对。原来源文件未改。错误摘录不是新增硬件证据，不收入本仓库。本文使用上表核对后的行号。

定点公开检索没有取得HPM6360用户手册。HPM6300数据手册Rev2.8的官方候选链接下载失败，未打开内容、无hash，不作为证据。无检索结果或下载失败不证明资料不存在。

波形、DMA取消顺序、ELF、全调用图、WCET、构建及测试的原始记录未在本次文档提交中生成。历史本地交接记录不冒充公开可点击验收材料。缺保证见[厂商问题清单](vendor-questions.md)，清单未发送。
