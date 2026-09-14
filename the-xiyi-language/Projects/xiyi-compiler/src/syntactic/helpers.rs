// src/syntactic/helpers.rs
//
// 纯工具函数的位置。这次从 parser.rs 拆出来的每一个函数都能明确归到
// 某个具体的语法域（item/func/expr/stmt/type/generic/attr/pattern/
// model/literal），暂时没有真正"谁都不属于、纯粹是通用小工具"的函数
// 需要放在这里——比如 module.rs 里的 peek/next/expect 这些，虽然也是
// "纯工具"，但它们是 Parser 自身最基础的原语，跟 struct Parser 的定义
// 放在同一个文件更自然，不适合搬到这里。
//
// 之所以仍然建这个文件（而不是等真正有需要时再建），是跟 check_attr.rs
// 那次一样的考虑：以后随便一个新的语法域文件里，如果冒出一个"这段代码
// 不属于我这个域，但也说不清该去哪"的函数，这里就是现成的落点，不用
// 到时候临时决定要不要新建文件。
