pkgname=fluxter-git
pkgver=r0.0000000
pkgrel=1
pkgdesc="A terminal-based chat client for the Fluxer messaging platform (git version)"
arch=('x86_64')
url="https://github.com/AIVirtuoso/fluxter"
license=('GPL-3.0-or-later')
depends=('gcc-libs')
makedepends=('cargo' 'git')
provides=('fluxter')
conflicts=('fluxter')
options=('!lto')
source=("fluxter::git+https://github.com/AIVirtuoso/fluxter.git")
sha256sums=('SKIP')

pkgver() {
    cd "$srcdir/fluxter"
    printf "r%s.%s" \
        "$(git rev-list --count HEAD)" \
        "$(git rev-parse --short=7 HEAD)"
}

build() {
    cd "$srcdir/fluxter"
    export RUSTFLAGS="${RUSTFLAGS} -C link-arg=-fuse-ld=bfd"
    cargo build --release --locked
}

package() {
    cd "$srcdir/fluxter"

    install -Dm755 \
        "target/release/fluxter" \
        "$pkgdir/usr/bin/fluxter"

    install -Dm644 \
        "README.md" \
        "$pkgdir/usr/share/doc/$pkgname/README.md"
}
