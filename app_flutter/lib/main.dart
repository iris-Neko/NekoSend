import 'dart:async';
import 'dart:convert';
import 'dart:developer' as developer;

import 'package:flutter/material.dart';

import 'application/app_controller.dart';
import 'application/models.dart';
import 'platform/platform_bootstrap.dart';
import 'presentation/lan_chat_app.dart';
import 'src/rust/api/core.dart';
import 'src/rust/api/health.dart';
import 'src/rust/events.dart';
import 'src/rust/frb_generated.dart';

Future<void> main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();

  CoreRuntimeInfo coreRuntime;
  NearbyPeerLoader? nearbyPeerLoader;
  ConversationLoader? conversationLoader;
  MessageLoader? messageLoader;
  MessageDeliveryLoader? messageDeliveryLoader;
  OpenPrivateConversation? openPrivateConversationCommand;
  SendTextCommand? sendTextCommand;
  TransferLoader? transferLoader;
  SourcePicker? sourcePicker;
  SendSourceCommand? sendSourceCommand;
  DecideIncomingOfferCommand? decideIncomingOfferCommand;
  TransferCommand? pauseTransferCommand;
  TransferCommand? resumeTransferCommand;
  TransferCommand? cancelTransferCommand;
  ClearCompletedTransfersCommand? clearCompletedTransfersCommand;
  OpenReferenceCommand? openReferenceCommand;
  PickAndSendSourceCommand? pickAndSendSourceCommand;
  AcceptIncomingOfferCommand? acceptIncomingOfferCommand;
  GroupInvitationLoader? groupInvitationLoader;
  CreateGroupCommand? createGroupCommand;
  GroupInviteDecisionCommand? groupInviteDecisionCommand;
  GroupLoader? groupLoader;
  GroupUpdateCommand? groupUpdateCommand;
  LeaveGroupCommand? leaveGroupCommand;
  OwnDeviceBindingLoader? ownDeviceBindingLoader;
  RequestOwnDeviceBindingCommand? requestOwnDeviceBindingCommand;
  DecideOwnDeviceBindingCommand? decideOwnDeviceBindingCommand;
  RemoveOwnDeviceBindingCommand? removeOwnDeviceBindingCommand;
  SetClipboardModeCommand? setClipboardModeCommand;
  SubmitClipboardTextCommand? submitClipboardTextCommand;
  SendClipboardMessageCommand? sendClipboardMessageCommand;
  ClipboardReader? clipboardReader;
  ClipboardImageReader? clipboardImageReader;
  SendClipboardImageCommand? sendClipboardImageCommand;
  ReselectTransferSourceCommand? reselectTransferSourceCommand;
  SettingsLoader? settingsLoader;
  SettingsUpdater? settingsUpdater;
  DeviceNameUpdater? deviceNameUpdater;
  PeerReceivePolicyLoader? peerReceivePolicyLoader;
  PeerReceivePolicyUpdater? peerReceivePolicyUpdater;
  SystemNotificationCommand? systemNotificationCommand;
  MarkConversationReadCommand? markConversationReadCommand;
  DeleteConversationCommand? deleteConversationCommand;
  PlatformRequestPump? platformRequestPump;
  CoreEventStreamFactory? coreEventStreamFactory;
  try {
    await RustLib.init();
    final health = getCoreHealth();
    final bootstrap = await PlatformBootstrap.load();
    var started = startCore(
      databasePath: bootstrap.databasePath,
      deviceName: bootstrap.deviceName,
      platform: bootstrap.platform,
      enableDiscovery: true,
    );
    if (bootstrap.legacyDeviceName != null) {
      final upgradedName = bootstrap.defaultNameUpgradeFor(started.deviceName);
      if (upgradedName != null) {
        started = updateDeviceName(
          clientOperationId: generateClientOperationId(),
          deviceName: upgradedName,
        );
      }
      await PlatformBootstrap.completeDeviceNameMigration();
    }
    coreRuntime = CoreRuntimeInfo.ready(
      version: health.coreVersion,
      protocolVersion: health.protocolVersion,
      deviceName: started.deviceName,
      deviceId: started.deviceId,
      discoveryAvailable: started.discoveryAvailable,
      discoveryError: started.discoveryError,
    );
    if (bootstrap.platform == 'android') {
      final savedReceiveTree = await PlatformBootstrap.getSavedReceiveTree();
      if (savedReceiveTree != null) {
        setDefaultReceiveRef(
          clientOperationId: generateClientOperationId(),
          receiveRef: savedReceiveTree,
        );
      }
    }
    var persistedSettings = getAppSettings();
    if (bootstrap.platform != 'android' &&
        persistedSettings.defaultReceiveRef == null) {
      final directory = await PlatformBootstrap.getDefaultReceiveDirectory();
      if (directory != null) {
        persistedSettings = updateAppSettings(
          clientOperationId: generateClientOperationId(),
          defaultReceiveRef: directory,
          clearDefaultReceiveRef: false,
        );
      }
    }
    await PlatformBootstrap.applyAppSettings(
      notificationsEnabled: persistedSettings.notificationsEnabled,
      closeToTray: persistedSettings.closeToTray,
      startOnBoot: persistedSettings.startOnBoot,
      androidKeepOnline: persistedSettings.androidKeepOnline,
    );

    AppSettingsView loadSettingsView() {
      final profile = getLocalProfile();
      if (profile == null) throw StateError('Rust 核心尚未启动');
      final settings = getAppSettings();
      return AppSettingsView(
        deviceName: profile.deviceName,
        deviceId: profile.deviceId,
        platform: profile.platform,
        defaultReceivePolicy: settings.defaultReceivePolicy,
        defaultReceiveRef: settings.defaultReceiveRef,
        notificationsEnabled: settings.notificationsEnabled,
        closeToTray: settings.closeToTray,
        startOnBoot: settings.startOnBoot,
        androidKeepOnline: settings.androidKeepOnline,
        autoOpenReceiveDirectory: settings.autoOpenReceiveDirectory,
        logLevel: settings.logLevel,
      );
    }

    settingsLoader = loadSettingsView;
    peerReceivePolicyLoader = () => listPeerReceivePolicies()
        .map(
          (peer) => PeerReceivePolicyView(
            deviceId: peer.deviceId,
            deviceName: peer.deviceName,
            relation: peer.relation,
            override: peer.receivePolicyOverride,
            effectivePolicy: peer.effectiveReceivePolicy,
          ),
        )
        .toList(growable: false);
    peerReceivePolicyUpdater = (peerDeviceId, receivePolicyOverride) {
      setPeerReceivePolicy(
        clientOperationId: generateClientOperationId(),
        peerDeviceId: peerDeviceId,
        receivePolicyOverride: receivePolicyOverride,
      );
    };
    deviceNameUpdater = (deviceName) {
      updateDeviceName(
        clientOperationId: generateClientOperationId(),
        deviceName: deviceName,
      );
      return loadSettingsView();
    };
    settingsUpdater = (next) async {
      final previous = getAppSettings();
      final updated = updateAppSettings(
        clientOperationId: generateClientOperationId(),
        defaultReceivePolicy: next.defaultReceivePolicy,
        defaultReceiveRef: next.defaultReceiveRef,
        clearDefaultReceiveRef: next.defaultReceiveRef == null,
        notificationsEnabled: next.notificationsEnabled,
        closeToTray: next.closeToTray,
        startOnBoot: next.startOnBoot,
        androidKeepOnline: next.androidKeepOnline,
        autoOpenReceiveDirectory: next.autoOpenReceiveDirectory,
        logLevel: next.logLevel,
      );
      try {
        if (bootstrap.platform == 'android' &&
            updated.androidKeepOnline &&
            !previous.androidKeepOnline) {
          await PlatformBootstrap.requestNotificationPermission();
        }
        await PlatformBootstrap.applyAppSettings(
          notificationsEnabled: updated.notificationsEnabled,
          closeToTray: updated.closeToTray,
          startOnBoot: updated.startOnBoot,
          androidKeepOnline: updated.androidKeepOnline,
        );
      } catch (_) {
        final restored = updateAppSettings(
          clientOperationId: generateClientOperationId(),
          defaultReceivePolicy: previous.defaultReceivePolicy,
          defaultReceiveRef: previous.defaultReceiveRef,
          clearDefaultReceiveRef: previous.defaultReceiveRef == null,
          notificationsEnabled: previous.notificationsEnabled,
          closeToTray: previous.closeToTray,
          startOnBoot: previous.startOnBoot,
          androidKeepOnline: previous.androidKeepOnline,
          autoOpenReceiveDirectory: previous.autoOpenReceiveDirectory,
          logLevel: previous.logLevel,
        );
        await PlatformBootstrap.applyAppSettings(
          notificationsEnabled: restored.notificationsEnabled,
          closeToTray: restored.closeToTray,
          startOnBoot: restored.startOnBoot,
          androidKeepOnline: restored.androidKeepOnline,
        );
        rethrow;
      }
      return loadSettingsView();
    };
    systemNotificationCommand =
        ({required title, required body, conversationId}) =>
            PlatformBootstrap.showSystemNotification(
              title: title,
              body: body,
              conversationId: conversationId,
            );
    openReferenceCommand = (reference, showInFolder) =>
        PlatformBootstrap.openReference(reference, showInFolder: showInFolder);
    List<OwnDeviceBindingView> loadBindingViews() => listOwnDeviceBindings()
        .map(
          (binding) => OwnDeviceBindingView(
            id: binding.bindingId,
            peerDeviceId: binding.peerDeviceId,
            peerName: binding.peerName,
            state: binding.state,
            clipboardMode: binding.clipboardMode,
            incoming: binding.requestedByDeviceId != started.deviceId,
            online: binding.online,
            createdAtMs: binding.createdAtMs,
          ),
        )
        .toList(growable: false);
    nearbyPeerLoader = () {
      final bindingByPeer = {
        for (final binding in loadBindingViews()) binding.peerDeviceId: binding,
      };
      return getNearbyPeers()
          .map(
            (peer) => NearbyDevice(
              id: peer.deviceId,
              name: peer.deviceName,
              platform: switch (peer.platform) {
                'android' => DevicePlatform.android,
                'linux' => DevicePlatform.linux,
                'macos' => DevicePlatform.macos,
                _ => DevicePlatform.windows,
              },
              relationLabel: switch (bindingByPeer[peer.deviceId]?.state) {
                'active' => '我的设备',
                'pending_outbound' => '绑定请求已发送',
                'pending_inbound' => '等待你确认',
                _ => null,
              },
              address: peer.sourceIp,
            ),
          )
          .toList(growable: false);
    };
    ownDeviceBindingLoader = loadBindingViews;
    requestOwnDeviceBindingCommand = (peerDeviceId) {
      requestOwnDeviceBinding(
        clientOperationId: generateClientOperationId(),
        peerDeviceId: peerDeviceId,
      );
    };
    decideOwnDeviceBindingCommand = (bindingId, accept) {
      decideOwnDeviceBinding(
        clientOperationId: generateClientOperationId(),
        bindingId: bindingId,
        accept: accept,
      );
    };
    removeOwnDeviceBindingCommand = (peerDeviceId) {
      removeOwnDeviceBinding(
        clientOperationId: generateClientOperationId(),
        peerDeviceId: peerDeviceId,
      );
    };
    setClipboardModeCommand = (peerDeviceId, mode) {
      setClipboardMode(
        clientOperationId: generateClientOperationId(),
        peerDeviceId: peerDeviceId,
        mode: mode,
      );
    };
    submitClipboardTextCommand = (peerDeviceId, text, automatic) {
      submitLocalClipboardText(
        clientOperationId: generateClientOperationId(),
        peerDeviceId: peerDeviceId,
        text: text,
        automatic: automatic,
      );
    };
    clipboardReader = PlatformBootstrap.readClipboardText;
    clipboardImageReader = () async {
      final picked = await PlatformBootstrap.readClipboardImage();
      if (picked == null) return null;
      final source = picked.sources.single;
      return ClipboardImageSelection(
        displayName: picked.displayName,
        sourceRef: source.sourceRef!,
        relativePath: source.relativePath,
        size: source.size,
        modifiedAtMs: source.modifiedAtMs,
        fingerprint: picked.fingerprint!,
      );
    };
    var lastAndroidActiveTransferCount = -1;
    var syncingAndroidActiveTransfers = false;
    var repeatAndroidActiveTransferSync = false;
    Future<void> syncAndroidActiveTransfers() async {
      if (bootstrap.platform != 'android') return;
      if (syncingAndroidActiveTransfers) {
        repeatAndroidActiveTransferSync = true;
        return;
      }
      syncingAndroidActiveTransfers = true;
      try {
        do {
          repeatAndroidActiveTransferSync = false;
          final count = listTransfers()
              .where(
                (transfer) => const {
                  'accepted',
                  'transferring',
                  'verifying',
                }.contains(transfer.state),
              )
              .length;
          if (count != lastAndroidActiveTransferCount) {
            await PlatformBootstrap.updateActiveTransferCount(count);
            lastAndroidActiveTransferCount = count;
          }
        } while (repeatAndroidActiveTransferSync);
      } finally {
        syncingAndroidActiveTransfers = false;
      }
    }

    var transferProgressUiEnabled = true;
    coreEventStreamFactory = () => subscribeCoreEvents()
        .where((event) {
          return transferProgressUiEnabled ||
              event.kind != CoreEventKind.transferProgress;
        })
        .map((event) {
          final update = _coreEventUpdate(event);
          if (update.area == CoreEventArea.transfer ||
              update.area == CoreEventArea.all) {
            unawaited(syncAndroidActiveTransfers());
          }
          return update;
        });
    Future<void> pauseAllTransfers() async {
      for (final transfer in listTransfers()) {
        if (!const {
          'queued',
          'accepted',
          'transferring',
        }.contains(transfer.state)) {
          continue;
        }
        try {
          pauseTransfer(
            clientOperationId: generateClientOperationId(),
            transferId: transfer.transferId,
          );
        } catch (_) {
          // A transfer may complete between the list and pause commands.
        }
      }
    }

    platformRequestPump = PlatformRequestPump.start(
      onLocalClipboardText: (text) async {
        for (final binding in listOwnDeviceBindings(state: 'active')) {
          if (binding.clipboardMode != 'send_only' &&
              binding.clipboardMode != 'bidirectional') {
            continue;
          }
          try {
            submitLocalClipboardText(
              clientOperationId: generateClientOperationId(),
              peerDeviceId: binding.peerDeviceId,
              text: text,
              automatic: true,
            );
          } catch (_) {
            // A later clipboard change or reconnect can retry for this device.
          }
        }
      },
      onLocalClipboardImage: (image) async {
        for (final binding in listOwnDeviceBindings(state: 'active')) {
          if (binding.clipboardMode != 'send_only' &&
              binding.clipboardMode != 'bidirectional') {
            continue;
          }
          try {
            final conversation = openPrivateConversation(
              clientOperationId: generateClientOperationId(),
              peerDeviceId: binding.peerDeviceId,
            );
            sendClipboardImageItems(
              clientOperationId: generateClientOperationId(),
              conversationId: conversation.conversationId,
              displayName: image.displayName,
              sources: image.sources,
              contentFingerprint: image.fingerprint!,
              automatic: true,
            );
          } catch (_) {
            // Continue syncing to the remaining bound devices.
          }
        }
      },
      onTraySendClipboard: (text) async {
        for (final binding in listOwnDeviceBindings(state: 'active')) {
          try {
            submitLocalClipboardText(
              clientOperationId: generateClientOperationId(),
              peerDeviceId: binding.peerDeviceId,
              text: text,
              automatic: false,
            );
          } catch (_) {
            // Continue sending to the remaining bound devices.
          }
        }
      },
      onTraySendClipboardImage: (image) async {
        for (final binding in listOwnDeviceBindings(state: 'active')) {
          try {
            final conversation = openPrivateConversation(
              clientOperationId: generateClientOperationId(),
              peerDeviceId: binding.peerDeviceId,
            );
            sendClipboardImageItems(
              clientOperationId: generateClientOperationId(),
              conversationId: conversation.conversationId,
              displayName: image.displayName,
              sources: image.sources,
              contentFingerprint: image.fingerprint!,
              automatic: false,
            );
          } catch (_) {
            // Continue sending to the remaining bound devices.
          }
        }
      },
      onPauseAllTransfers: pauseAllTransfers,
      onStopAndroidOnline: () async {
        shutdownCore();
      },
      onSystemAndroidOnlineTimeout: () async {
        shutdownCore();
      },
      onNetworkChanged: () async {
        shutdownCore();
        startCore(
          databasePath: bootstrap.databasePath,
          deviceName: bootstrap.deviceName,
          platform: bootstrap.platform,
          enableDiscovery: true,
        );
        lastAndroidActiveTransferCount = -1;
        await syncAndroidActiveTransfers();
      },
    );
    unawaited(syncAndroidActiveTransfers());
    ConversationSummary mapConversation(ConversationDto conversation) =>
        ConversationSummary(
          id: conversation.conversationId,
          title: conversation.title,
          preview: conversation.lastMessagePreview ?? '开始聊天',
          timeLabel: _timeLabel(conversation.lastActivityAtMs),
          isGroup: conversation.isGroup,
          online: conversation.online,
          peerDeviceId: conversation.peerDeviceId,
          unreadCount: conversation.unreadCount,
          memberCount: conversation.memberCount,
          onlineCount: conversation.onlineCount,
        );
    ChatMessage mapMessage(MessageDto message) {
      final totalSize = message.totalSize;
      final persistedBytes = message.persistedBytes;
      return ChatMessage(
        id: message.messageId,
        senderDeviceId: message.senderDeviceId,
        content: message.text,
        timeLabel: _timeLabel(message.createdAtMs),
        outgoing: message.outgoing,
        localSortOrder: message.localSortOrder,
        deliveredCount: message.deliveredCount,
        deliveryCount: message.deliveryCount,
        kind: _messageKind(message.kind),
        statusLabel: message.conversationId.startsWith('g:') && message.outgoing
            ? _groupMessageStatus(
                message.state,
                message.deliveredCount,
                message.deliveryCount,
              )
            : message.transferState == null
            ? message.outgoing
                  ? _messageStatus(message.state)
                  : ''
            : _transferStatus(message.transferState!),
        fileSizeLabel: totalSize == null
            ? null
            : '${message.entryCount ?? 1} 项 · ${_formatBytes(totalSize)}',
        localFileRef: message.localFileRef,
        progress: totalSize == null || persistedBytes == null
            ? null
            : totalSize == BigInt.zero
            ? (message.transferState == 'completed' ? 1 : 0)
            : (persistedBytes.toDouble() / totalSize.toDouble()).clamp(0, 1),
      );
    }

    conversationLoader = () =>
        listConversations().map(mapConversation).toList(growable: false);
    messageLoader = (conversationId) => listMessages(
      conversationId: conversationId,
      limit: 100,
    ).map(mapMessage).toList(growable: false);
    messageDeliveryLoader = (messageId) =>
        listMessageDeliveries(messageId: messageId)
            .map(
              (delivery) => MessageDeliveryView(
                recipientDeviceId: delivery.recipientDeviceId,
                recipientName: delivery.recipientName,
                state: delivery.state,
                failureReason: delivery.failureReason,
                updatedAtMs: delivery.updatedAtMs,
                delivered: delivery.delivered,
              ),
            )
            .toList(growable: false);
    openPrivateConversationCommand = (device) => mapConversation(
      openPrivateConversation(
        clientOperationId: generateClientOperationId(),
        peerDeviceId: device.id,
      ),
    );
    groupInvitationLoader = () => listGroupInvitations()
        .map(
          (invitation) => GroupInvitationView(
            id: invitation.inviteId,
            groupId: invitation.groupId,
            groupName: invitation.groupName,
            inviterDeviceId: invitation.inviterDeviceId,
            inviterName: invitation.inviterName,
            memberDeviceIds: invitation.memberDeviceIds,
            createdAtMs: invitation.createdAtMs,
          ),
        )
        .toList(growable: false);
    createGroupCommand = (name, memberDeviceIds) {
      final result = createGroup(
        clientOperationId: generateClientOperationId(),
        name: name,
        memberDeviceIds: memberDeviceIds,
      );
      return mapConversation(
        listConversations().firstWhere(
          (conversation) =>
              conversation.conversationId == result.conversationId,
        ),
      );
    };
    groupInviteDecisionCommand = (inviteId, accept) => decideGroupInvite(
      clientOperationId: generateClientOperationId(),
      inviteId: inviteId,
      accept: accept,
    );
    groupLoader = (groupId) {
      final group = getGroup(groupId: groupId);
      return GroupView(
        id: group.groupId,
        name: group.name,
        ownerDeviceId: group.ownerDeviceId,
        revision: group.revision.toInt(),
        state: group.state,
        localRole: group.localRole,
        localMembership: group.localMembership,
        members: group.members
            .map(
              (member) => GroupMemberView(
                deviceId: member.deviceId,
                role: member.role,
                membership: member.membership,
                online: member.online,
              ),
            )
            .toList(growable: false),
      );
    };
    groupUpdateCommand =
        (
          groupId,
          expectedRevision,
          name,
          addMemberIds,
          removeMemberIds,
          transferOwnerTo,
          disband,
        ) {
          updateGroup(
            clientOperationId: generateClientOperationId(),
            groupId: groupId,
            expectedRevision: BigInt.from(expectedRevision),
            name: name,
            addMemberIds: addMemberIds,
            removeMemberIds: removeMemberIds,
            transferOwnerTo: transferOwnerTo,
            disband: disband,
          );
        };
    leaveGroupCommand = (groupId) => leaveGroup(
      clientOperationId: generateClientOperationId(),
      groupId: groupId,
    );
    sendTextCommand = (conversationId, text) => mapMessage(
      sendTextMessage(
        clientOperationId: generateClientOperationId(),
        conversationId: conversationId,
        text: text,
      ),
    );
    sendClipboardMessageCommand = (conversationId, text) => mapMessage(
      sendClipboardTextMessage(
        clientOperationId: generateClientOperationId(),
        conversationId: conversationId,
        text: text,
      ),
    );
    sendClipboardImageCommand = (conversationId, image) {
      final sent = sendClipboardImageItems(
        clientOperationId: generateClientOperationId(),
        conversationId: conversationId,
        displayName: image.displayName,
        sources: [
          SourceItemDto(
            entryKind: 'file',
            sourceRef: image.sourceRef,
            relativePath: image.relativePath,
            size: image.size,
            modifiedAtMs: image.modifiedAtMs,
          ),
        ],
        contentFingerprint: image.fingerprint,
        automatic: false,
      );
      return mapMessage(
        listMessages(
          conversationId: conversationId,
          limit: 100,
        ).firstWhere((message) => message.messageId == sent.messageId),
      );
    };
    markConversationReadCommand = (conversationId, throughSortOrder) {
      markConversationRead(
        clientOperationId: generateClientOperationId(),
        conversationId: conversationId,
        throughSortOrder: throughSortOrder,
      );
    };
    deleteConversationCommand = (conversationId) {
      deleteConversation(
        clientOperationId: generateClientOperationId(),
        conversationId: conversationId,
      );
    };
    transferLoader = () => listTransfers()
        .map(
          (transfer) => TransferView(
            id: transfer.transferId,
            messageId: transfer.messageId,
            conversationId: transfer.conversationId,
            peerDeviceId: transfer.peerDeviceId,
            direction: transfer.direction,
            state: transfer.state,
            failureReason: transfer.failureReason,
            displayName: transfer.displayName,
            totalSize: transfer.totalSize,
            entryCount: transfer.entryCount,
            persistedBytes: transfer.persistedBytes,
            receiveBaseRef: transfer.receiveBaseRef,
            localFileRef: transfer.localFileRef,
            pausedByUser: transfer.pausedByUser,
          ),
        )
        .toList(growable: false);
    clearCompletedTransfersCommand = () =>
        clearCompletedTransfers(clientOperationId: generateClientOperationId())
            .toInt();
    sourcePicker = PlatformBootstrap.pickSource;
    sendSourceCommand = (conversationId, sourcePath, messageKind) {
      sendSourcePath(
        clientOperationId: generateClientOperationId(),
        conversationId: conversationId,
        sourcePath: sourcePath,
        messageKind: messageKind,
      );
    };
    _registerDebugExtensions(
      sendTextCommand,
      setTransferProgressUiEnabled: (enabled) {
        transferProgressUiEnabled = enabled;
      },
    );
    decideIncomingOfferCommand = (transferId, accept, receiveBaseRef) {
      decideIncomingOffer(
        clientOperationId: generateClientOperationId(),
        transferId: transferId,
        accept: accept,
        receiveBaseRef: receiveBaseRef,
      );
    };
    pauseTransferCommand = (transferId) => pauseTransfer(
      clientOperationId: generateClientOperationId(),
      transferId: transferId,
    );
    resumeTransferCommand = (transferId) => resumeTransfer(
      clientOperationId: generateClientOperationId(),
      transferId: transferId,
    );
    cancelTransferCommand = (transferId) => cancelTransfer(
      clientOperationId: generateClientOperationId(),
      transferId: transferId,
    );
    pickAndSendSourceCommand = (conversationId, messageKind) async {
      if (bootstrap.platform == 'android') {
        final picked = await PlatformBootstrap.pickSafSource(messageKind);
        if (picked == null) return false;
        sendSourceItems(
          clientOperationId: generateClientOperationId(),
          conversationId: conversationId,
          displayName: picked.displayName,
          messageKind: messageKind,
          sources: picked.sources,
        );
        return true;
      }
      final path = await PlatformBootstrap.pickSource(messageKind);
      if (path == null) return false;
      sendSourcePath(
        clientOperationId: generateClientOperationId(),
        conversationId: conversationId,
        sourcePath: path,
        messageKind: messageKind,
      );
      return true;
    };
    acceptIncomingOfferCommand = (transferId) async {
      if (bootstrap.platform == 'android') {
        final prepared = await PlatformBootstrap.prepareSafReceive(
          listTransferEntries(transferId: transferId),
        );
        if (prepared == null) return false;
        decideIncomingOfferPrepared(
          clientOperationId: generateClientOperationId(),
          transferId: transferId,
          receiveBaseRef: prepared.receiveBaseRef,
          prepared: prepared.prepared,
        );
        setDefaultReceiveRef(
          clientOperationId: generateClientOperationId(),
          receiveRef: prepared.receiveBaseRef,
        );
        return true;
      }
      final directory = await PlatformBootstrap.pickSource('folder');
      if (directory == null) return false;
      decideIncomingOffer(
        clientOperationId: generateClientOperationId(),
        transferId: transferId,
        accept: true,
        receiveBaseRef: directory,
      );
      return true;
    };
    reselectTransferSourceCommand = (transfer) async {
      final entries = listTransferEntries(transferId: transfer.id);
      if (transfer.needsDestinationReselection) {
        if (bootstrap.platform == 'android') {
          final prepared = await PlatformBootstrap.pickSafReceive(entries);
          if (prepared == null) return false;
          replaceReceiveDestinationPrepared(
            clientOperationId: generateClientOperationId(),
            transferId: transfer.id,
            receiveBaseRef: prepared.receiveBaseRef,
            prepared: prepared.prepared,
          );
          setDefaultReceiveRef(
            clientOperationId: generateClientOperationId(),
            receiveRef: prepared.receiveBaseRef,
          );
          return true;
        }
        final directory = await PlatformBootstrap.pickSource('folder');
        if (directory == null) return false;
        replaceReceiveDestinationPath(
          clientOperationId: generateClientOperationId(),
          transferId: transfer.id,
          receiveBaseRef: directory,
        );
        setDefaultReceiveRef(
          clientOperationId: generateClientOperationId(),
          receiveRef: directory,
        );
        return true;
      }
      final kind = entries.any((entry) => entry.entryKind == 'directory')
          ? 'folder'
          : 'file';
      if (bootstrap.platform == 'android') {
        final picked = await PlatformBootstrap.pickSafSource(kind);
        if (picked == null) return false;
        replaceTransferSourceItems(
          clientOperationId: generateClientOperationId(),
          transferId: transfer.id,
          sources: picked.sources,
        );
        return true;
      }
      final path = await PlatformBootstrap.pickSource(kind);
      if (path == null) return false;
      replaceTransferSourcePath(
        clientOperationId: generateClientOperationId(),
        transferId: transfer.id,
        sourcePath: path,
      );
      return true;
    };
  } catch (error) {
    coreRuntime = CoreRuntimeInfo.failed(_presentCoreError(error));
  }

  runApp(
    LanChatApp(
      coreRuntime: coreRuntime,
      nearbyPeerLoader: nearbyPeerLoader,
      conversationLoader: conversationLoader,
      messageLoader: messageLoader,
      messageDeliveryLoader: messageDeliveryLoader,
      openPrivateConversation: openPrivateConversationCommand,
      sendTextCommand: sendTextCommand,
      transferLoader: transferLoader,
      sourcePicker: sourcePicker,
      sendSourceCommand: sendSourceCommand,
      decideIncomingOfferCommand: decideIncomingOfferCommand,
      pauseTransferCommand: pauseTransferCommand,
      resumeTransferCommand: resumeTransferCommand,
      cancelTransferCommand: cancelTransferCommand,
      clearCompletedTransfersCommand: clearCompletedTransfersCommand,
      openReferenceCommand: openReferenceCommand,
      pickAndSendSourceCommand: pickAndSendSourceCommand,
      acceptIncomingOfferCommand: acceptIncomingOfferCommand,
      groupInvitationLoader: groupInvitationLoader,
      createGroupCommand: createGroupCommand,
      groupInviteDecisionCommand: groupInviteDecisionCommand,
      groupLoader: groupLoader,
      groupUpdateCommand: groupUpdateCommand,
      leaveGroupCommand: leaveGroupCommand,
      ownDeviceBindingLoader: ownDeviceBindingLoader,
      requestOwnDeviceBindingCommand: requestOwnDeviceBindingCommand,
      decideOwnDeviceBindingCommand: decideOwnDeviceBindingCommand,
      removeOwnDeviceBindingCommand: removeOwnDeviceBindingCommand,
      setClipboardModeCommand: setClipboardModeCommand,
      submitClipboardTextCommand: submitClipboardTextCommand,
      sendClipboardMessageCommand: sendClipboardMessageCommand,
      clipboardReader: clipboardReader,
      clipboardImageReader: clipboardImageReader,
      sendClipboardImageCommand: sendClipboardImageCommand,
      reselectTransferSourceCommand: reselectTransferSourceCommand,
      settingsLoader: settingsLoader,
      settingsUpdater: settingsUpdater,
      deviceNameUpdater: deviceNameUpdater,
      deviceIdentityLoader: () => listDeviceIdentities()
          .map(
            (item) => DeviceIdentityView(
              deviceId: item.deviceId,
              deviceName: item.deviceName,
              avatarId: item.avatarId,
              isLocal: item.isLocal,
            ),
          )
          .toList(growable: false),
      deviceAvatarUpdater: (avatarId) => setDeviceAvatar(
        clientOperationId: generateClientOperationId(),
        avatarId: avatarId,
      ),
      peerReceivePolicyLoader: peerReceivePolicyLoader,
      peerReceivePolicyUpdater: peerReceivePolicyUpdater,
      systemNotificationCommand: systemNotificationCommand,
      markConversationReadCommand: markConversationReadCommand,
      deleteConversationCommand: deleteConversationCommand,
      platformRequestPump: platformRequestPump,
      coreEventStreamFactory: coreEventStreamFactory,
      initialForegroundAction: arguments
          .where((argument) => argument.startsWith('openConversation:'))
          .firstOrNull,
      errorPresenter: _presentCoreError,
    ),
  );
}

void _registerDebugExtensions(
  SendTextCommand command, {
  required void Function(bool enabled) setTransferProgressUiEnabled,
}) {
  assert(() {
    bool boolParameter(Map<String, String> parameters, String name) =>
        parameters[name]?.toLowerCase() == 'true';

    String requiredParameter(Map<String, String> parameters, String name) {
      final value = parameters[name];
      if (value == null || value.isEmpty) {
        throw ArgumentError('$name is required');
      }
      return value;
    }

    developer.registerExtension('ext.lanChat.sendText', (_, parameters) async {
      try {
        final conversationId = parameters['conversationId'];
        final text = parameters['text'];
        if (conversationId == null || text == null) {
          throw ArgumentError('conversationId and text are required');
        }
        final sent = command(conversationId, text);
        return developer.ServiceExtensionResponse.result(
          jsonEncode({'messageId': sent.id, 'status': sent.statusLabel}),
        );
      } catch (error) {
        return developer.ServiceExtensionResponse.error(
          developer.ServiceExtensionResponse.extensionError,
          error.toString(),
        );
      }
    });
    developer.registerExtension('ext.lanChat.sendSourcePath', (
      _,
      parameters,
    ) async {
      try {
        final conversationId = parameters['conversationId'];
        final sourcePath = parameters['sourcePath'];
        final messageKind = parameters['messageKind'] ?? 'file';
        if (conversationId == null || sourcePath == null) {
          throw ArgumentError('conversationId and sourcePath are required');
        }
        final sent = sendSourcePath(
          clientOperationId: generateClientOperationId(),
          conversationId: conversationId,
          sourcePath: sourcePath,
          messageKind: messageKind,
        );
        return developer.ServiceExtensionResponse.result(
          jsonEncode({
            'messageId': sent.messageId,
            'transferId': sent.transferId,
          }),
        );
      } catch (error) {
        return developer.ServiceExtensionResponse.error(
          developer.ServiceExtensionResponse.extensionError,
          error.toString(),
        );
      }
    });
    developer.registerExtension('ext.lanChat.debugCommand', (
      _,
      parameters,
    ) async {
      try {
        final action = requiredParameter(parameters, 'action');
        Object? result;
        switch (action) {
          case 'snapshot':
            final profile = getLocalProfile();
            final settings = getAppSettings();
            result = {
              'profile': profile == null
                  ? null
                  : {
                      'deviceId': profile.deviceId,
                      'deviceName': profile.deviceName,
                      'platform': profile.platform,
                      'discoveryAvailable': profile.discoveryAvailable,
                    },
              'nearby': getNearbyPeers()
                  .map(
                    (peer) => {
                      'deviceId': peer.deviceId,
                      'deviceName': peer.deviceName,
                      'platform': peer.platform,
                      'sourceIp': peer.sourceIp,
                    },
                  )
                  .toList(),
              'bindings': listOwnDeviceBindings()
                  .map(
                    (binding) => {
                      'bindingId': binding.bindingId,
                      'peerDeviceId': binding.peerDeviceId,
                      'peerName': binding.peerName,
                      'state': binding.state,
                      'clipboardMode': binding.clipboardMode,
                      'online': binding.online,
                    },
                  )
                  .toList(),
              'identities': listDeviceIdentities()
                  .map(
                    (identity) => {
                      'deviceId': identity.deviceId,
                      'deviceName': identity.deviceName,
                      'avatarId': identity.avatarId,
                      'isLocal': identity.isLocal,
                    },
                  )
                  .toList(),
              'groupInvitations': listGroupInvitations()
                  .map(
                    (invitation) => {
                      'inviteId': invitation.inviteId,
                      'groupId': invitation.groupId,
                      'groupName': invitation.groupName,
                      'inviterDeviceId': invitation.inviterDeviceId,
                    },
                  )
                  .toList(),
              'conversations': listConversations()
                  .map(
                    (conversation) => {
                      'conversationId': conversation.conversationId,
                      'title': conversation.title,
                      'peerDeviceId': conversation.peerDeviceId,
                      'groupId': conversation.groupId,
                      'isGroup': conversation.isGroup,
                      'online': conversation.online,
                    },
                  )
                  .toList(),
              'settings': {
                'defaultReceivePolicy': settings.defaultReceivePolicy,
                'defaultReceiveRef': settings.defaultReceiveRef,
                'androidKeepOnline': settings.androidKeepOnline,
              },
              'transfers': listTransfers()
                  .map(
                    (transfer) => {
                      'transferId': transfer.transferId,
                      'messageId': transfer.messageId,
                      'peerDeviceId': transfer.peerDeviceId,
                      'direction': transfer.direction,
                      'state': transfer.state,
                      'failureReason': transfer.failureReason,
                      'displayName': transfer.displayName,
                      'totalSize': transfer.totalSize.toString(),
                      'persistedBytes': transfer.persistedBytes.toString(),
                      'receiveBaseRef': transfer.receiveBaseRef,
                    },
                  )
                  .toList(),
            };
          case 'set_transfer_progress_ui':
            final enabled = boolParameter(parameters, 'enabled');
            setTransferProgressUiEnabled(enabled);
            result = {'enabled': enabled};
          case 'request_binding':
            result = {
              'bindingId': requestOwnDeviceBinding(
                clientOperationId: generateClientOperationId(),
                peerDeviceId: requiredParameter(parameters, 'peerDeviceId'),
              ),
            };
          case 'decide_binding':
            result = {
              'peerDeviceId': decideOwnDeviceBinding(
                clientOperationId: generateClientOperationId(),
                bindingId: requiredParameter(parameters, 'bindingId'),
                accept: boolParameter(parameters, 'accept'),
              ),
            };
          case 'remove_binding':
            removeOwnDeviceBinding(
              clientOperationId: generateClientOperationId(),
              peerDeviceId: requiredParameter(parameters, 'peerDeviceId'),
            );
            result = {'ok': true};
          case 'set_clipboard_mode':
            setClipboardMode(
              clientOperationId: generateClientOperationId(),
              peerDeviceId: requiredParameter(parameters, 'peerDeviceId'),
              mode: requiredParameter(parameters, 'mode'),
            );
            result = {'ok': true};
          case 'submit_clipboard':
            final sent = submitLocalClipboardText(
              clientOperationId: generateClientOperationId(),
              peerDeviceId: requiredParameter(parameters, 'peerDeviceId'),
              text: requiredParameter(parameters, 'text'),
              automatic: boolParameter(parameters, 'automatic'),
            );
            result = {
              'messageId': sent.messageId,
              'eventId': sent.eventId,
              'clipboardSequence': sent.clipboardSequence.toString(),
            };
          case 'send_text':
            final sent = command(
              requiredParameter(parameters, 'conversationId'),
              requiredParameter(parameters, 'text'),
            );
            result = {'messageId': sent.id, 'status': sent.statusLabel};
          case 'open_private_conversation':
            final conversation = openPrivateConversation(
              clientOperationId: generateClientOperationId(),
              peerDeviceId: requiredParameter(parameters, 'peerDeviceId'),
            );
            result = {'conversationId': conversation.conversationId};
          case 'send_source':
            final sent = sendSourcePath(
              clientOperationId: generateClientOperationId(),
              conversationId: requiredParameter(parameters, 'conversationId'),
              sourcePath: requiredParameter(parameters, 'sourcePath'),
              messageKind: requiredParameter(parameters, 'messageKind'),
            );
            result = {
              'messageId': sent.messageId,
              'transferId': sent.transferId,
            };
          case 'set_receive_ref':
            setDefaultReceiveRef(
              clientOperationId: generateClientOperationId(),
              receiveRef: requiredParameter(parameters, 'receiveRef'),
            );
            result = {'ok': true};
          case 'rename_device':
            final profile = updateDeviceName(
              clientOperationId: generateClientOperationId(),
              deviceName: requiredParameter(parameters, 'deviceName'),
            );
            result = {
              'deviceId': profile.deviceId,
              'deviceName': profile.deviceName,
              'discoveryAvailable': profile.discoveryAvailable,
            };
          case 'set_avatar':
            result = {
              'avatarId': setDeviceAvatar(
                clientOperationId: generateClientOperationId(),
                avatarId: requiredParameter(parameters, 'avatarId'),
              ),
            };
          case 'clear_receive_ref':
            setDefaultReceiveRef(
              clientOperationId: generateClientOperationId(),
              receiveRef: null,
            );
            result = {'ok': true};
          case 'pick_receive_directory':
            result = {
              'receiveRef': await PlatformBootstrap.pickReceiveDirectory(),
            };
          case 'replace_receive_destination':
            final transferId = requiredParameter(parameters, 'transferId');
            final receiveRef = requiredParameter(parameters, 'receiveRef');
            final prepared = await PlatformBootstrap.prepareSafReceiveAt(
              receiveRef,
              listTransferEntries(transferId: transferId),
            );
            replaceReceiveDestinationPrepared(
              clientOperationId: generateClientOperationId(),
              transferId: transferId,
              receiveBaseRef: prepared.receiveBaseRef,
              prepared: prepared.prepared,
            );
            setDefaultReceiveRef(
              clientOperationId: generateClientOperationId(),
              receiveRef: prepared.receiveBaseRef,
            );
            result = {'ok': true};
          case 'pause_transfer':
            pauseTransfer(
              clientOperationId: generateClientOperationId(),
              transferId: requiredParameter(parameters, 'transferId'),
            );
            result = {'ok': true};
          case 'resume_transfer':
            resumeTransfer(
              clientOperationId: generateClientOperationId(),
              transferId: requiredParameter(parameters, 'transferId'),
            );
            result = {'ok': true};
          case 'cancel_transfer':
            cancelTransfer(
              clientOperationId: generateClientOperationId(),
              transferId: requiredParameter(parameters, 'transferId'),
            );
            result = {'ok': true};
          case 'exit_application':
            result = {'ok': true};
            unawaited(
              Future<void>.delayed(const Duration(milliseconds: 100), () async {
                shutdownCore();
                await PlatformBootstrap.exitApplication();
              }),
            );
          case 'create_group':
            final members = requiredParameter(
              parameters,
              'memberDeviceIds',
            ).split(',').where((value) => value.isNotEmpty).toList();
            final created = createGroup(
              clientOperationId: generateClientOperationId(),
              name: requiredParameter(parameters, 'name'),
              memberDeviceIds: members,
            );
            result = {
              'groupId': created.groupId,
              'conversationId': created.conversationId,
              'inviteIds': created.inviteIds,
            };
          case 'decide_group_invite':
            result = {
              'groupId': decideGroupInvite(
                clientOperationId: generateClientOperationId(),
                inviteId: requiredParameter(parameters, 'inviteId'),
                accept: boolParameter(parameters, 'accept'),
              ),
            };
          case 'get_group':
            final group = getGroup(
              groupId: requiredParameter(parameters, 'groupId'),
            );
            result = {
              'groupId': group.groupId,
              'name': group.name,
              'ownerDeviceId': group.ownerDeviceId,
              'revision': group.revision.toString(),
              'state': group.state,
              'localRole': group.localRole,
              'localMembership': group.localMembership,
              'members': group.members
                  .map(
                    (member) => {
                      'deviceId': member.deviceId,
                      'role': member.role,
                      'membership': member.membership,
                      'online': member.online,
                    },
                  )
                  .toList(),
            };
          case 'update_group':
            List<String> listParameter(String name) => (parameters[name] ?? '')
                .split(',')
                .where((value) => value.isNotEmpty)
                .toList();
            final updated = updateGroup(
              clientOperationId: generateClientOperationId(),
              groupId: requiredParameter(parameters, 'groupId'),
              expectedRevision: BigInt.parse(
                requiredParameter(parameters, 'expectedRevision'),
              ),
              name: parameters['name']?.isEmpty == false
                  ? parameters['name']
                  : null,
              addMemberIds: listParameter('addMemberIds'),
              removeMemberIds: listParameter('removeMemberIds'),
              transferOwnerTo: parameters['transferOwnerTo']?.isEmpty == false
                  ? parameters['transferOwnerTo']
                  : null,
              disband: boolParameter(parameters, 'disband'),
            );
            result = {
              'groupId': updated.groupId,
              'revision': updated.revision.toString(),
              'inviteIds': updated.inviteIds,
            };
          case 'leave_group':
            leaveGroup(
              clientOperationId: generateClientOperationId(),
              groupId: requiredParameter(parameters, 'groupId'),
            );
            result = {'ok': true};
          case 'list_messages':
            result = {
              'messages':
                  listMessages(
                        conversationId: requiredParameter(
                          parameters,
                          'conversationId',
                        ),
                        limit: 100,
                      )
                      .map(
                        (message) => {
                          'messageId': message.messageId,
                          'senderDeviceId': message.senderDeviceId,
                          'kind': message.kind,
                          'state': message.state,
                          'text': message.text,
                          'outgoing': message.outgoing,
                        },
                      )
                      .toList(),
            };
          default:
            throw ArgumentError('unknown debug action: $action');
        }
        return developer.ServiceExtensionResponse.result(jsonEncode(result));
      } catch (error) {
        return developer.ServiceExtensionResponse.error(
          developer.ServiceExtensionResponse.extensionError,
          error.toString(),
        );
      }
    });
    return true;
  }());
}

String _presentCoreError(Object error) {
  try {
    return describeCoreError(message: error.toString()).userMessage;
  } catch (_) {
    return '操作失败，请重试';
  }
}

CoreEventUpdate _coreEventUpdate(CoreEventDto event) {
  final progress = event.transferProgress;
  if (event.kind == CoreEventKind.transferProgress && progress != null) {
    return CoreEventUpdate(
      CoreEventArea.transfer,
      transferProgress: TransferProgressUpdate(
        transferId: progress.transferId,
        state: progress.state,
        persistedBytes: progress.persistedBytes,
        totalSize: progress.totalSize,
        bytesPerSecond: progress.bytesPerSecond,
        etaSeconds: progress.etaSeconds,
        activeEntryRelativePath: progress.activeEntryRelativePath,
      ),
    );
  }
  return CoreEventUpdate(switch (event.kind) {
    CoreEventKind.peerPresenceChanged => CoreEventArea.presence,
    CoreEventKind.conversationChanged ||
    CoreEventKind.messageChanged => CoreEventArea.conversation,
    CoreEventKind.transferProgress ||
    CoreEventKind.incomingOfferRequiresDecision => CoreEventArea.transfer,
    CoreEventKind.groupInviteReceived => CoreEventArea.invitation,
    CoreEventKind.ownDeviceBindingRequested => CoreEventArea.binding,
    CoreEventKind.coreReady ||
    CoreEventKind.appSnapshotInvalidated ||
    CoreEventKind.platformRequest ||
    CoreEventKind.notificationRequested ||
    CoreEventKind.coreErrorOccurred => CoreEventArea.all,
  });
}

MessageVisualKind _messageKind(String kind) => switch (kind) {
  'file' => MessageVisualKind.file,
  'image' || 'clipboard_image' => MessageVisualKind.image,
  'folder' => MessageVisualKind.folder,
  'clipboard_text' => MessageVisualKind.clipboard,
  _ => MessageVisualKind.text,
};

String _transferStatus(String state) => switch (state) {
  'queued' => '等待对方上线',
  'offered' => '等待接收',
  'accepted' => '准备传输',
  'transferring' => '传输中',
  'paused' => '已暂停',
  'verifying' => '正在完成',
  'completed' => '已完成',
  'rejected' => '对方已拒绝',
  'cancelled' => '已取消',
  _ => '传输失败',
};

String _formatBytes(BigInt bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  final digits = unit == 0 || value >= 100 ? 0 : 1;
  return '${value.toStringAsFixed(digits)} ${units[unit]}';
}

String _messageStatus(String state) => switch (state) {
  'delivered' => '已送达',
  'sending' => '正在发送',
  'failed' => '发送失败',
  _ => '等待发送',
};

String _groupMessageStatus(String state, int delivered, int total) {
  if (total == 0) return '无投递目标';
  if (state == 'failed') return '发送失败 0/$total';
  if (delivered == 0) return '等待送达 0/$total';
  return '已送达 $delivered/$total';
}

String _timeLabel(int milliseconds) {
  final time = DateTime.fromMillisecondsSinceEpoch(milliseconds);
  final now = DateTime.now();
  if (time.year == now.year && time.month == now.month && time.day == now.day) {
    return '${time.hour.toString().padLeft(2, '0')}:'
        '${time.minute.toString().padLeft(2, '0')}';
  }
  return '${time.month}/${time.day}';
}
