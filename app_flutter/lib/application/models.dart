enum SidebarSection { conversations, nearby }

enum MainContent { conversation, transfers }

enum DevicePlatform { windows, android, linux }

enum MessageVisualKind { text, file, image, folder, clipboard }

class AppSettingsView {
  const AppSettingsView({
    required this.deviceName,
    required this.deviceId,
    required this.platform,
    required this.defaultReceivePolicy,
    required this.defaultReceiveRef,
    required this.notificationsEnabled,
    required this.closeToTray,
    required this.startOnBoot,
    required this.androidKeepOnline,
    this.autoOpenReceiveDirectory = false,
    required this.logLevel,
  });

  final String deviceName;
  final String deviceId;
  final String platform;
  final String defaultReceivePolicy;
  final String? defaultReceiveRef;
  final bool notificationsEnabled;
  final bool closeToTray;
  final bool startOnBoot;
  final bool androidKeepOnline;
  final bool autoOpenReceiveDirectory;
  final String logLevel;

  AppSettingsView copyWith({
    String? deviceName,
    String? defaultReceivePolicy,
    String? defaultReceiveRef,
    bool clearDefaultReceiveRef = false,
    bool? notificationsEnabled,
    bool? closeToTray,
    bool? startOnBoot,
    bool? androidKeepOnline,
    bool? autoOpenReceiveDirectory,
    String? logLevel,
  }) => AppSettingsView(
    deviceName: deviceName ?? this.deviceName,
    deviceId: deviceId,
    platform: platform,
    defaultReceivePolicy: defaultReceivePolicy ?? this.defaultReceivePolicy,
    defaultReceiveRef: clearDefaultReceiveRef
        ? null
        : defaultReceiveRef ?? this.defaultReceiveRef,
    notificationsEnabled: notificationsEnabled ?? this.notificationsEnabled,
    closeToTray: closeToTray ?? this.closeToTray,
    startOnBoot: startOnBoot ?? this.startOnBoot,
    androidKeepOnline: androidKeepOnline ?? this.androidKeepOnline,
    autoOpenReceiveDirectory:
        autoOpenReceiveDirectory ?? this.autoOpenReceiveDirectory,
    logLevel: logLevel ?? this.logLevel,
  );
}

class PeerReceivePolicyView {
  const PeerReceivePolicyView({
    required this.deviceId,
    required this.deviceName,
    required this.relation,
    required this.override,
    required this.effectivePolicy,
  });

  final String deviceId;
  final String deviceName;
  final String relation;
  final String? override;
  final String effectivePolicy;
}

class ConversationSummary {
  const ConversationSummary({
    required this.id,
    required this.title,
    required this.preview,
    required this.timeLabel,
    required this.isGroup,
    required this.online,
    this.peerDeviceId,
    this.unreadCount = 0,
    this.transferProgress,
    this.memberCount = 2,
    this.onlineCount = 0,
  });

  final String id;
  final String title;
  final String preview;
  final String timeLabel;
  final bool isGroup;
  final bool online;
  final String? peerDeviceId;
  final int unreadCount;
  final double? transferProgress;
  final int memberCount;
  final int onlineCount;

  ConversationSummary copyWith({int? unreadCount}) => ConversationSummary(
    id: id,
    title: title,
    preview: preview,
    timeLabel: timeLabel,
    isGroup: isGroup,
    online: online,
    peerDeviceId: peerDeviceId,
    unreadCount: unreadCount ?? this.unreadCount,
    transferProgress: transferProgress,
    memberCount: memberCount,
    onlineCount: onlineCount,
  );
}

class GroupInvitationView {
  const GroupInvitationView({
    required this.id,
    required this.groupId,
    required this.groupName,
    required this.inviterDeviceId,
    required this.inviterName,
    required this.memberDeviceIds,
    required this.createdAtMs,
  });

  final String id;
  final String groupId;
  final String groupName;
  final String inviterDeviceId;
  final String inviterName;
  final List<String> memberDeviceIds;
  final int createdAtMs;
}

class GroupMemberView {
  const GroupMemberView({
    required this.deviceId,
    required this.role,
    required this.membership,
    required this.online,
  });

  final String deviceId;
  final String role;
  final String membership;
  final bool online;
}

class GroupView {
  const GroupView({
    required this.id,
    required this.name,
    required this.ownerDeviceId,
    required this.revision,
    required this.state,
    required this.localRole,
    required this.localMembership,
    required this.members,
  });

  final String id;
  final String name;
  final String ownerDeviceId;
  final int revision;
  final String state;
  final String localRole;
  final String localMembership;
  final List<GroupMemberView> members;

  bool get isOwner => localRole == 'owner' && localMembership == 'joined';
}

class NearbyDevice {
  const NearbyDevice({
    required this.id,
    required this.name,
    required this.platform,
    required this.relationLabel,
    this.address,
  });

  final String id;
  final String name;
  final DevicePlatform platform;
  final String? relationLabel;
  final String? address;
}

class OwnDeviceBindingView {
  const OwnDeviceBindingView({
    required this.id,
    required this.peerDeviceId,
    required this.peerName,
    required this.state,
    required this.clipboardMode,
    required this.incoming,
    required this.online,
    required this.createdAtMs,
  });

  final String id;
  final String peerDeviceId;
  final String peerName;
  final String state;
  final String clipboardMode;
  final bool incoming;
  final bool online;
  final int createdAtMs;

  bool get active => state == 'active';
  bool get pendingInbound => state == 'pending_inbound' && incoming;
}

class ChatMessage {
  const ChatMessage({
    required this.id,
    required this.content,
    required this.timeLabel,
    required this.outgoing,
    required this.kind,
    required this.statusLabel,
    this.fileSizeLabel,
    this.localFileRef,
    this.progress,
    this.localSortOrder = 0,
    this.deliveredCount = 0,
    this.deliveryCount = 0,
  });

  final String id;
  final String content;
  final String timeLabel;
  final bool outgoing;
  final MessageVisualKind kind;
  final String statusLabel;
  final String? fileSizeLabel;
  final String? localFileRef;
  final double? progress;
  final int localSortOrder;
  final int deliveredCount;
  final int deliveryCount;
}

class MessageDeliveryView {
  const MessageDeliveryView({
    required this.recipientDeviceId,
    required this.recipientName,
    required this.state,
    required this.updatedAtMs,
    required this.delivered,
    this.failureReason,
  });

  final String recipientDeviceId;
  final String recipientName;
  final String state;
  final String? failureReason;
  final int updatedAtMs;
  final bool delivered;
}

class ClipboardImageSelection {
  const ClipboardImageSelection({
    required this.displayName,
    required this.sourceRef,
    required this.relativePath,
    required this.size,
    required this.modifiedAtMs,
    required this.fingerprint,
  });

  final String displayName;
  final String sourceRef;
  final String relativePath;
  final BigInt size;
  final int modifiedAtMs;
  final String fingerprint;
}

class TransferView {
  TransferView({
    required this.id,
    required this.messageId,
    this.conversationId = '',
    required this.peerDeviceId,
    required this.direction,
    required this.state,
    required this.displayName,
    required this.totalSize,
    required this.entryCount,
    required this.persistedBytes,
    required this.pausedByUser,
    this.failureReason,
    this.receiveBaseRef,
    this.localFileRef,
    BigInt? bytesPerSecond,
    this.etaSeconds,
    this.activeEntryRelativePath,
  }) : bytesPerSecond = bytesPerSecond ?? BigInt.zero;

  final String id;
  final String messageId;
  final String conversationId;
  final String peerDeviceId;
  final String direction;
  final String state;
  final String? failureReason;
  final String displayName;
  final BigInt totalSize;
  final int entryCount;
  final BigInt persistedBytes;
  final String? receiveBaseRef;
  final String? localFileRef;
  final bool pausedByUser;
  final BigInt bytesPerSecond;
  final BigInt? etaSeconds;
  final String? activeEntryRelativePath;

  TransferView copyWith({
    String? state,
    BigInt? totalSize,
    BigInt? persistedBytes,
    BigInt? bytesPerSecond,
    BigInt? etaSeconds,
    bool clearEtaSeconds = false,
    String? activeEntryRelativePath,
    bool clearActiveEntryRelativePath = false,
  }) => TransferView(
    id: id,
    messageId: messageId,
    conversationId: conversationId,
    peerDeviceId: peerDeviceId,
    direction: direction,
    state: state ?? this.state,
    failureReason: failureReason,
    displayName: displayName,
    totalSize: totalSize ?? this.totalSize,
    entryCount: entryCount,
    persistedBytes: persistedBytes ?? this.persistedBytes,
    receiveBaseRef: receiveBaseRef,
    localFileRef: localFileRef,
    pausedByUser: pausedByUser,
    bytesPerSecond: bytesPerSecond ?? this.bytesPerSecond,
    etaSeconds: clearEtaSeconds ? null : etaSeconds ?? this.etaSeconds,
    activeEntryRelativePath: clearActiveEntryRelativePath
        ? null
        : activeEntryRelativePath ?? this.activeEntryRelativePath,
  );

  bool get awaitsDecision => direction == 'receive' && state == 'offered';

  bool get needsSourceReselection =>
      direction == 'send' &&
      state == 'failed' &&
      const {'source_changed', 'permission_lost'}.contains(failureReason);

  bool get needsDestinationReselection =>
      direction == 'receive' &&
      state == 'failed' &&
      const {
        'not_enough_space',
        'permission_lost',
        'invalid_path',
      }.contains(failureReason);

  bool get needsReselection =>
      needsSourceReselection || needsDestinationReselection;

  double get progress {
    if (totalSize == BigInt.zero) return state == 'completed' ? 1 : 0;
    return (persistedBytes.toDouble() / totalSize.toDouble()).clamp(0, 1);
  }
}
