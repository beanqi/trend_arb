


/// * 等待指定的时间，直到当前时间减去上次更新时间大于指定的连接等待时间
/// * @param connect_wait_time: 连接等待时间
/// * @param last_update_time: 上次更新时间
/// * @return: None
/// * @example: wait_util_ms(1000, last_update_time).await;
pub async fn wait_util_ms<'a>(connect_wait_time: u16, last_update_time: u64) {
    let now_ms = tokio::time::Instant::now().elapsed().as_millis() as u64;

    if now_ms - last_update_time < connect_wait_time as u64 {
        // 如果连接时间间隔小于最小等待时间，则等待
        tokio::time::sleep(
            tokio::time::Duration::from_millis(
                connect_wait_time as u64 - (now_ms - last_update_time),
            ),
        ).await;
    }
}

/// 单线程共享数据结构，用于tokio运行时
/// 使用本结构必须保证同时只能有一个线程访问/操作这个结构
pub struct SingleThreadSharedData<T> {
    ptr: *mut T,
    // 使用 PhantomData 告诉编译器，我们“拥有”一个 T 类型的数据
    // 这对于 Drop Check 和类型变异（Variance）是重要的
    _phantom: std::marker::PhantomData<T>,
}

// 这里需要对 T 添加 Trait Bound：
// - T 必须是 Send，才能让 UnsafeSharedData<T> 在线程间传递
// - T 必须是 Sync，才能让 &UnsafeSharedData<T> 在线程间共享
unsafe impl<T: Send> Send for SingleThreadSharedData<T> {}
unsafe impl<T: Sync> Sync for SingleThreadSharedData<T> {}

impl<T> SingleThreadSharedData<T> {
    // 创建一个新的实例，接收一个泛型 T 的值
    pub fn new(data: T) -> Self {
        Self {
            ptr: Box::into_raw(Box::new(data)),
            _phantom: std::marker::PhantomData,
        }
    }

    // 获取对 T 的可变引用
    pub fn get_mut(&self) -> &mut T {
        // Safety: 这里我们只在单线程中使用这个结构
        unsafe { &mut *self.ptr }
    }

    // 获取不可变引用
    pub fn get(&self) -> &T {
        // Safety: 这里我们只在单线程中使用这个结构
        unsafe { &*self.ptr }
    }
}

// 4. 实现 Drop trait 来为泛型 T 释放内存
impl<T> Drop for SingleThreadSharedData<T> {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // 将裸指针转换回 Box<T>，然后让 Box 自动 drop
            unsafe {
                let _ = Box::from_raw(self.ptr);
            }
        }
    }
}