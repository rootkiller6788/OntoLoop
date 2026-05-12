TUI Design Guidelines
Rust + Ratatui 终端界面设计规范
打造高辨识度、生产级、极简但精致的终端界面，拒绝 generic 终端 UI、拒绝千篇一律的默认样式。
Core Philosophy
TUI 不是简陋的界面，它是轻量化、高性能、原生级、无依赖的产品界面。设计目标：克制、精准、有记忆点、工业级美感。
每一个界面必须有明确的审美方向，不做随机 / 默认 / AI 风样式。
Design Thinking (TUI 版)
在写代码前，先确定清晰、大胆、一致的设计方向：
Purpose：界面解决什么问题？用户是谁？
Tone：选定一种极端风格并贯彻到底
Brutalist / Raw（粗野主义）
Industrial / Minimal（工业极简）
Luxury / Refined（精致克制）
Editorial / Structured（编辑式排版）
Retro / 80s terminal（复古终端）
Soft / Monochrome（柔和单色）
Constraints：TUI 天生限制 = 字符、颜色、布局、性能
Differentiation：界面必须有一个让人记住的亮点
例如：独特边框、渐变字符、不对称布局、动态排版、专属图标
关键：意图大于复杂度。极简做到极致，比杂乱更高级。
TUI Aesthetics Guidelines (Ratatui 专用)
1. Typography（字符排版）
放弃默认杂乱对齐，使用严格网格系统
标题使用 BLOCK / UPPERCASE / DOUBLE_WIDTH 强化层级
正文保持 等宽对齐、紧凑留白、高可读性
拒绝随机大小写、随机缩进
用符号图标替代文字（● ■ ▶ ◆ ↪）提升质感
2. Color System（颜色系统）
使用 ANSI 256 + 真彩色 打造层次感
主色 + 强调色 最多 3 种，避免花里胡哨
深色主题优先：黑灰底 + 高饱和强调色最有高级感
使用 CSS 风格的常量系统统一颜色
拒绝：默认蓝、默认紫、AI 最爱渐变
3. Layout & Composition（布局）
坚持 不对称、有呼吸感、有重点
用区块分割创造秩序，而不是线条堆砌
允许元素重叠、跨栏、突破网格制造记忆点
大量使用 空白（Negative Space） 提升精致度
布局必须稳定、可预测、不抖动
4. Borders & Blocks（边框与区块）
选择 一种边框风格贯穿全局
THICK
ROUNDED
MINIMAL
BRUTALIST
NONE
拒绝混合边框风格
用阴影、空白、缩进替代多余线条
5. Motion & Interactivity（动效与交互）
TUI 动效追求 轻、快、干脆
支持：
进入动画（staggered reveal）
选择高亮
状态切换
加载指示器
拒绝：过度闪烁、频繁刷新、花里胡哨
6. Visual Details（视觉质感）
TUI 的高级感来自细节：
精致分隔符
有序列表符号
状态徽章（SUCCESS / ERROR / LOADING）
轻微灰度渐变
统一的图标语言
像素级对齐
Rules to Avoid Generic "AI TUI Slop"
严格禁止以下风格，保持独特性：
默认线条、默认颜色、默认布局
到处都是边框、到处都是分隔线
无层次、无重点、无呼吸感
千篇一律的蓝色 / 紫色主题
随机字体大小写
无意义的动画
布局拥挤、信息爆炸
每个 TUI 项目必须拥有独特视觉识别系统。
Implementation Principles (Rust + Ratatui)
代码结构清晰，一个模块一个界面
样式抽成 const 变量，便于统一修改
布局使用 Layout + Constraint 实现响应式
状态管理干净，不混乱
渲染性能优先：避免不必要重绘
一个 cargo run 即可运行完整 TUI
What We’re Building
Distinctive, production-grade, memorable terminal interfaces.不是简陋控制台，而是原生级、无依赖、极简美学的产品界面。
