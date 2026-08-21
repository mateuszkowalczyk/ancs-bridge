pkgname=ancs-bridge
pkgver=0.1.0
pkgrel=1
pkgdesc='Secure local iPhone notification forwarding for Linux via Apple ANCS'
arch=('x86_64')
url='https://github.com/mateuszkowalczyk/ancs-bridge'
license=('MIT')
options=('!debug')
depends=('bluez' 'dbus' 'wireplumber')
makedepends=('cargo')
source=("$pkgname-$pkgver.tar.gz::https://github.com/mateuszkowalczyk/ancs-bridge/archive/refs/tags/v${pkgver}.tar.gz")
sha256sums=('936d88b31a4675d11d349fd6b6a498f459a2ccb82a7b21927a47c111b8c8515a')

prepare() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
}

build() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo build --release --locked --frozen --offline
}

check() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo test --all-targets --locked --frozen --offline
}

package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 target/release/ancs-bridge "$pkgdir/usr/bin/ancs-bridge"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 packaging/ancs-bridge.service \
    "$pkgdir/usr/lib/systemd/user/ancs-bridge.service"
}
