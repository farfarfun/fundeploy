//! 把一批互不相干的活并发做完，结果按输入顺序回来。
//!
//! 这个程序里反复出现同一种形状：有 N 件事要做，彼此没有依赖，每件都在**等**
//! （等网络、等子进程），串行做就是把等待时间加起来。换源测速、GitHub 连通性
//! 诊断本来各写了一遍线程池，技能仓库同步和开发工具链状态是第三、第四处——
//! 与其抄第四遍，不如把这十几行放在一处。
//!
//! 两条约束让它值得单独成文件，而不是每处随手 `spawn` 一把：
//!
//! 1. **结果必须按输入顺序。** 按完成顺序收的话，「最快的那个源」配出去的会是
//!    另一个源的地址，而这种错不会报错，只会悄悄配错。
//! 2. **抢任务，不按线程数均分。** 均分的话一件慢活会拖住和它分到同一份的其他
//!    活；而这些场景里快慢差一个数量级是常态（本地仓库 10 ms，跨境仓库 3 s）。
//!
//! 不引线程池 crate：标准库的 `thread::scope` 就够，而且借用检查替我们保证了
//! 线程在函数返回前一定收完。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// 并发上限的默认值。
///
/// 这些活全在等 IO，不占 CPU，所以不照核数定。8 是在「跨境网络要靠并发把延迟叠
/// 起来」和「别把对面的服务器当压测目标」之间取的——技能仓库同步会对同一个
/// Gitee 组织连 8 条 git，再多就不礼貌了。
pub const JOBS: usize = 8;

/// 对 `items` 里每一项并发跑 `f`，返回值与 `items` 一一对应、同序。
///
/// `workers` 会被收进 `1..=items.len()`，所以传 0 或者传个比项数大得多的值都不会
/// 出事（空输入直接返回空）。
pub fn map<T, R, F>(items: &[T], workers: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    if items.is_empty() {
        return Vec::new();
    }
    let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let workers = workers.clamp(1, items.len());

    std::thread::scope(|s| {
        for _ in 0..workers {
            let (slots, next, f) = (&slots, &next, &f);
            s.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= items.len() {
                    break;
                }
                let r = f(&items[i]);
                *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            });
        }
    });

    // scope 退出时线程全都 join 过了，每个槽必定被写过一次。
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("每个槽都该被写过")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    #[test]
    fn 结果按输入顺序而不是完成顺序() {
        // 这条是这个模块存在的首要理由。故意让下标越小的睡得越久，按完成顺序
        // 收的话结果正好是倒过来的。
        let items: Vec<u64> = vec![40, 30, 20, 10];
        let got = map(&items, 4, |&ms| {
            std::thread::sleep(Duration::from_millis(ms));
            ms
        });
        assert_eq!(got, items);
    }

    #[test]
    fn 每一项都跑到且只跑一次() {
        let items: Vec<usize> = (0..50).collect();
        let calls = AtomicUsize::new(0);
        let got = map(&items, 8, |&i| {
            calls.fetch_add(1, Ordering::Relaxed);
            i * 2
        });
        assert_eq!(calls.load(Ordering::Relaxed), 50);
        assert_eq!(got, (0..50).map(|i| i * 2).collect::<Vec<_>>());
    }

    #[test]
    fn 一件慢活不拖住其余的() {
        // 抢任务而不是均分的意义：第 0 项很慢，剩下的该被其他线程领走。
        // 均分的话（4 个线程各拿 4 项）第 0 个线程要串行背完自己那四项。
        let items: Vec<u64> = std::iter::once(300)
            .chain(std::iter::repeat_n(80, 15))
            .collect();
        let t = std::time::Instant::now();
        map(&items, 4, |&ms| {
            std::thread::sleep(Duration::from_millis(ms))
        });
        let el = t.elapsed();
        // 串行要 1.5 s；均分到 4 份时第一份是 300+80*3=540 ms。抢任务应该在
        // 400 ms 上下。放宽到 900 ms，只要能把「退化成均分」判出来就够。
        assert!(
            el < Duration::from_millis(900),
            "用了 {el:?}，像是没在抢任务"
        );
    }

    #[test]
    fn 空输入不起线程也不报错() {
        let empty: Vec<u8> = Vec::new();
        let got: Vec<u8> = map(&empty, 8, |&x| x);
        assert!(got.is_empty());
    }

    #[test]
    fn 并发数比项数多或者为零都不出事() {
        let items = vec![1, 2, 3];
        assert_eq!(map(&items, 99, |&x| x), items);
        assert_eq!(map(&items, 0, |&x| x), items);
    }
}
