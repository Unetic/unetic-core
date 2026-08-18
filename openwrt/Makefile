include $(TOPDIR)/rules.mk

PKG_NAME:=unetic-core
PKG_VERSION:=$(shell sed -n 's/^version = "\([^"]*\)"/\1/p' $(CURDIR)/source/Cargo.toml | head -n1)
PKG_RELEASE:=1
PKG_LICENSE:=GPL-2.0-only
PKG_LICENSE_FILES:=LICENSE
PKG_BUILD_DIR:=$(BUILD_DIR)/$(PKG_NAME)-$(PKG_VERSION)

include $(INCLUDE_DIR)/package.mk

define Package/unetic-core
  SECTION:=admin
  CATEGORY:=Administration
  TITLE:=Unetic control-plane daemon
  URL:=https://github.com/Unetic/unetic-core
  DEPENDS:=+rpcd +libubus +libubox +libblobmsg-json
endef

define Package/unetic-core/description
 Unetic control-plane daemon for OpenWrt.
endef

define Build/Prepare
	mkdir -p $(PKG_BUILD_DIR)
	$(CP) $(CURDIR)/source/. $(PKG_BUILD_DIR)/
endef

define Build/Compile
	$(TARGET_CC) $(TARGET_CFLAGS) -fPIC -shared \
		-I$(STAGING_DIR)/usr/include \
		$(PKG_BUILD_DIR)/openwrt/native/unetic_openwrt.c \
		-o $(PKG_BUILD_DIR)/libunetic-openwrt.so \
		$(TARGET_LDFLAGS) -lubus -lubox -lblobmsg_json
endef

define Package/unetic-core/install
	$(INSTALL_DIR) $(1)/usr/sbin
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/openwrt-binary $(1)/usr/sbin/unetic-core

	$(INSTALL_DIR) $(1)/usr/lib
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/libunetic-openwrt.so $(1)/usr/lib/libunetic-openwrt.so

	$(INSTALL_DIR) $(1)/etc/init.d
	$(INSTALL_BIN) ./files/unetic-core.init $(1)/etc/init.d/unetic-core

	$(INSTALL_DIR) $(1)/usr/share/rpcd/acl.d
	$(INSTALL_DATA) ./files/unetic.json $(1)/usr/share/rpcd/acl.d/unetic.json
endef

$(eval $(call BuildPackage,unetic-core))
