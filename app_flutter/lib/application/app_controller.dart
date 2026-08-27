import 'dart:async';

import 'package:flutter/foundation.dart';

import 'models.dart';

typedef NearbyPeerLoader = List<NearbyDevice> Function();
typedef ConversationLoader = List<ConversationSummary> Function();
typedef MessageLoader = List<ChatMessage> Function(String conversationId);
typedef MessageDeliveryLoader = List<MessageDeliveryView> Function(
  String messageId,
);
typedef OpenPrivateConversation = ConversationSummary Function(
  NearbyDevice device,
);
typedef SendTextCommand = ChatMessage Function(
  String conversationId,
  String text,
);
typedef TransferLoader = List<TransferView> Function();
typedef SourcePicker = Future<String?> Function(String kind);
typedef SendSourceCommand = void Function(
  String conversationId,
  String sourcePath,
  String messageKind,
);
typedef DecideIncomingOfferCommand = void Function(
  String transferId,
  bool accept,
  String? receiveBaseRef,
);
typedef TransferCommand = void Function(String transferId);
typedef ClearCompletedTransfersCommand = int Function();
typedef OpenReferenceCommand = Future<void> Function(
  String reference,
  bool showInFolder,
);
typedef PickAndSendSourceCommand = Future<bool> Function(
  String conversationId,
  String messageKind,
);
typedef AcceptIncomingOfferCommand = Future<bool> Function(String transferId);
typedef GroupInvitationLoader = List<GroupInvitationView> Function();
typedef CreateGroupCommand = ConversationSummary Function(
  String name,
  List<String> memberDeviceIds,
);
typedef GroupInviteDecisionCommand = String? Function(
  String inviteId,
  bool accept,
);
typedef GroupLoader = GroupView Function(String groupId);
typedef GroupUpdateCommand = void Function(
  String groupId,
  int expectedRevision,
  String? name,
  List<String> addMemberIds,
  List<String> removeMemberIds,
  String? transferOwnerTo,
  bool disband,
);
typedef LeaveGroupCommand = void Function(String groupId);
typedef OwnDeviceBindingLoader = List<OwnDeviceBindingView> Function();
typedef RequestOwnDeviceBindingCommand = void Function(String peerDeviceId);
typedef DecideOwnDeviceBindingCommand = void Function(
  String bindingId,
  bool accept,
);
typedef RemoveOwnDeviceBindingCommand = void Function(String peerDeviceId);
typedef SetClipboardModeCommand = void Function(
  String peerDeviceId,
  String mode,
);
typedef SubmitClipboardTextCommand = void Function(
  String peerDeviceId,
  String text,
  bool automatic,
);
typedef SendClipboardMessageCommand = ChatMessage Function(
  String conversationId,
  String text,
);
typedef ClipboardReader = Future<String?> Function();
typedef ClipboardImageReader = Future<ClipboardImageSelection?> Function();
typedef SendClipboardImageCommand = ChatMessage Function(
  String conversationId,
  ClipboardImageSelection image,
);
typedef ReselectTransferSourceCommand = Future<bool> Function(
  TransferView transfer,
);
typedef SettingsLoader = AppSettingsView Function();
typedef SettingsUpdater = Future<AppSettingsView> Function(
  AppSettingsView settings,
);
typedef DeviceNameUpdater = AppSettingsView Function(String deviceName);
typedef PeerReceivePolicyLoader = List<PeerReceivePolicyView> Function();
typedef PeerReceivePolicyUpdater = void Function(
  String peerDeviceId,
  String? receivePolicyOverride,
);
typedef SystemNotificationCommand = Future<void> Function({
  required String title,
  required String body,
  String? conversationId,
});
typedef MarkConversationReadCommand = void Function(
  String conversationId,
  int throughSortOrder,
);
typedef DeleteConversationCommand = void Function(String conversationId);
typedef ErrorPresenter = String Function(Object error);

String defaultErrorPresenter(Object error) => error.toString();

enum CoreEventArea {
  all,
  presence,
  conversation,
  transfer,
  invitation,
  binding,
}

class TransferProgressUpdate {
  const TransferProgressUpdate({
    required this.transferId,
    required this.state,
    required this.persistedBytes,
    required this.totalSize,
    required this.bytesPerSecond,
    this.etaSeconds,
    this.activeEntryRelativePath,
  });

  final String transferId;
  final String state;
  final BigInt persistedBytes;
  final BigInt totalSize;
  final BigInt bytesPerSecond;
  final BigInt? etaSeconds;
  final String? activeEntryRelativePath;
}

class CoreEventUpdate {
  const CoreEventUpdate(this.area, {this.transferProgress});

  final CoreEventArea area;
  final TransferProgressUpdate? transferProgress;
}

typedef CoreEventStreamFactory = Stream<CoreEventUpdate> Function();

class AppController extends ChangeNotifier {
  AppController({
    this.nearbyPeerLoader,
    this.conversationLoader,
    List<NearbyDevice> initialNearbyDevices = const [],
    List<ConversationSummary> initialConversations = const [],
    Map<String, List<ChatMessage>> initialMessages = const {},
    this.messageLoader,
    this.messageDeliveryLoader,
    this.openPrivateConversation,
    this.sendTextCommand,
    this.transferLoader,
    this.sourcePicker,
    this.sendSourceCommand,
    this.decideIncomingOfferCommand,
    this.pauseTransferCommand,
    this.resumeTransferCommand,
    this.cancelTransferCommand,
    this.clearCompletedTransfersCommand,
    this.openReferenceCommand,
    this.pickAndSendSourceCommand,
    this.acceptIncomingOfferCommand,
    this.groupInvitationLoader,
    this.createGroupCommand,
    this.groupInviteDecisionCommand,
    this.groupLoader,
    this.groupUpdateCommand,
    this.leaveGroupCommand,
    this.ownDeviceBindingLoader,
    this.requestOwnDeviceBindingCommand,
    this.decideOwnDeviceBindingCommand,
    this.removeOwnDeviceBindingCommand,
    this.setClipboardModeCommand,
    this.submitClipboardTextCommand,
    this.sendClipboardMessageCommand,
    this.clipboardReader,
    this.clipboardImageReader,
    this.sendClipboardImageCommand,
    this.reselectTransferSourceCommand,
    this.settingsLoader,
    this.settingsUpdater,
    this.deviceNameUpdater,
    this.peerReceivePolicyLoader,
    this.peerReceivePolicyUpdater,
    this.systemNotificationCommand,
    this.markConversationReadCommand,
    this.deleteConversationCommand,
    this.coreEventStreamFactory,
    this.errorPresenter = defaultErrorPresenter,
  }) : nearbyDevices = List<NearbyDevice>.of(initialNearbyDevices),
       conversations = List<ConversationSummary>.of(initialConversations),
       selectedConversationId = initialConversations.isEmpty
           ? ''
           : initialConversations.first.id {
    _messagesByConversation = initialMessages.map(
      (conversationId, messages) =>
          MapEntry(conversationId, List<ChatMessage>.of(messages)),
    );
    if (nearbyPeerLoader != null) {
      refreshAll();
      if (coreEventStreamFactory == null) {
        _startSnapshotTimer(const Duration(seconds: 1));
      } else {
        _subscribeToCoreEvents();
      }
    }
  }

  final NearbyPeerLoader? nearbyPeerLoader;
  final ConversationLoader? conversationLoader;
  final MessageLoader? messageLoader;
  final MessageDeliveryLoader? messageDeliveryLoader;
  final OpenPrivateConversation? openPrivateConversation;
  final SendTextCommand? sendTextCommand;
  final TransferLoader? transferLoader;
  final SourcePicker? sourcePicker;
  final SendSourceCommand? sendSourceCommand;
  final DecideIncomingOfferCommand? decideIncomingOfferCommand;
  final TransferCommand? pauseTransferCommand;
  final TransferCommand? resumeTransferCommand;
  final TransferCommand? cancelTransferCommand;
  final ClearCompletedTransfersCommand? clearCompletedTransfersCommand;
  final OpenReferenceCommand? openReferenceCommand;
  final PickAndSendSourceCommand? pickAndSendSourceCommand;
  final AcceptIncomingOfferCommand? acceptIncomingOfferCommand;
  final GroupInvitationLoader? groupInvitationLoader;
  final CreateGroupCommand? createGroupCommand;
  final GroupInviteDecisionCommand? groupInviteDecisionCommand;
  final GroupLoader? groupLoader;
  final GroupUpdateCommand? groupUpdateCommand;
  final LeaveGroupCommand? leaveGroupCommand;
  final OwnDeviceBindingLoader? ownDeviceBindingLoader;
  final RequestOwnDeviceBindingCommand? requestOwnDeviceBindingCommand;
  final DecideOwnDeviceBindingCommand? decideOwnDeviceBindingCommand;
  final RemoveOwnDeviceBindingCommand? removeOwnDeviceBindingCommand;
  final SetClipboardModeCommand? setClipboardModeCommand;
  final SubmitClipboardTextCommand? submitClipboardTextCommand;
  final SendClipboardMessageCommand? sendClipboardMessageCommand;
  final ClipboardReader? clipboardReader;
  final ClipboardImageReader? clipboardImageReader;
  final SendClipboardImageCommand? sendClipboardImageCommand;
  final ReselectTransferSourceCommand? reselectTransferSourceCommand;
  final SettingsLoader? settingsLoader;
  final SettingsUpdater? settingsUpdater;
  final DeviceNameUpdater? deviceNameUpdater;
  final PeerReceivePolicyLoader? peerReceivePolicyLoader;
  final PeerReceivePolicyUpdater? peerReceivePolicyUpdater;
  final SystemNotificationCommand? systemNotificationCommand;
  final MarkConversationReadCommand? markConversationReadCommand;
  final DeleteConversationCommand? deleteConversationCommand;
  final CoreEventStreamFactory? coreEventStreamFactory;
  final ErrorPresenter errorPresenter;
  Timer? _nearbyTimer;
  Timer? _eventReconnectTimer;
  StreamSubscription<CoreEventUpdate>? _coreEventSubscription;
  bool _disposed = false;
  bool _notificationSnapshotInitialized = false;
  final Map<String, int> _lastUnreadCounts = {};
  final Set<String> _knownInvitationIds = {};
  final Set<String> _knownIncomingOfferIds = {};
  final Set<String> _knownCompletedTransferIds = {};

  final List<ConversationSummary> conversations;

  List<NearbyDevice> nearbyDevices;
  List<TransferView> transfers = const [];
  List<GroupInvitationView> groupInvitations = const [];
  List<OwnDeviceBindingView> ownDeviceBindings = const [];
  AppSettingsView? settings;
  List<PeerReceivePolicyView> peerReceivePolicies = const [];
  Object? _lastError;

  String? get lastError {
    final error = _lastError;
    if (error == null) return null;
    try {
      return errorPresenter(error);
    } catch (_) {
      return '操作失败，请重试';
    }
  }

  set lastError(String? value) => _lastError = value;

  late final Map<String, List<ChatMessage>> _messagesByConversation;
  SidebarSection section = SidebarSection.conversations;
  MainContent mainContent = MainContent.conversation;
  String selectedConversationId;
  String? focusedMessageId;
  String searchQuery = '';
  bool compactDetailVisible = false;

  ConversationSummary get selectedConversation => conversations.firstWhere(
    (conversation) => conversation.id == selectedConversationId,
    orElse: () => conversations.first,
  );

  bool get hasSelectedConversation =>
      conversations.any((item) => item.id == selectedConversationId);

  String? get selectedPeerDeviceId =>
      hasSelectedConversation ? selectedConversation.peerDeviceId : null;

  OwnDeviceBindingView? bindingForPeer(String peerDeviceId) {
    for (final binding in ownDeviceBindings) {
      if (binding.peerDeviceId == peerDeviceId && binding.state != 'removed') {
        return binding;
      }
    }
    return null;
  }

  List<OwnDeviceBindingView> get pendingOwnDeviceBindings => ownDeviceBindings
      .where((binding) => binding.pendingInbound)
      .toList(growable: false);

  List<OwnDeviceBindingView> get activeOwnDeviceBindings => ownDeviceBindings
      .where((binding) => binding.active)
      .toList(growable: false);

  List<ChatMessage> get selectedMessages => List.unmodifiable(
    _messagesByConversation[selectedConversationId] ?? const [],
  );

  List<MessageDeliveryView>? loadMessageDeliveries(String messageId) {
    final loader = messageDeliveryLoader;
    if (loader == null) return const [];
    try {
      lastError = null;
      return loader(messageId);
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return null;
    }
  }

  List<ConversationSummary> get filteredConversations {
    final query = searchQuery.trim().toLowerCase();
    if (query.isEmpty) return conversations;
    return conversations
        .where(
          (conversation) => conversation.title.toLowerCase().contains(query),
        )
        .toList(growable: false);
  }

  List<NearbyDevice> get filteredNearbyDevices {
    final query = searchQuery.trim().toLowerCase();
    if (query.isEmpty) return nearbyDevices;
    return nearbyDevices
        .where((device) => device.name.toLowerCase().contains(query))
        .toList(growable: false);
  }

  void setSection(SidebarSection value) {
    section = value;
    searchQuery = '';
    notifyListeners();
  }

  void setSearchQuery(String value) {
    searchQuery = value;
    notifyListeners();
  }

  void refreshNearby() {
    final loader = nearbyPeerLoader;
    if (loader == null) return;
    try {
      nearbyDevices = loader();
      notifyListeners();
    } catch (_) {
      // A transient core restart is corrected by the next one-second poll.
    }
  }

  void _subscribeToCoreEvents() {
    if (_disposed) return;
    final factory = coreEventStreamFactory;
    if (factory == null) return;
    _eventReconnectTimer?.cancel();
    _startSnapshotTimer(const Duration(seconds: 30));
    try {
      _coreEventSubscription?.cancel();
      _coreEventSubscription = factory().listen(
        _refreshForCoreEvent,
        onError: (_) => _handleCoreEventStreamEnded(),
        onDone: _handleCoreEventStreamEnded,
        cancelOnError: true,
      );
    } catch (_) {
      _handleCoreEventStreamEnded();
    }
  }

  void _handleCoreEventStreamEnded() {
    if (_disposed || coreEventStreamFactory == null) return;
    _startSnapshotTimer(const Duration(seconds: 1));
    _eventReconnectTimer?.cancel();
    _eventReconnectTimer = Timer(
      const Duration(seconds: 1),
      _subscribeToCoreEvents,
    );
  }

  void _startSnapshotTimer(Duration interval) {
    _nearbyTimer?.cancel();
    _nearbyTimer = Timer.periodic(interval, (_) => refreshAll());
  }

  void _refreshForCoreEvent(CoreEventUpdate event) {
    final progress = event.transferProgress;
    if (progress != null) {
      if (progress.state == 'completed') {
        refreshAll(
          includeNearby: false,
          includeInvitations: false,
          includeBindings: false,
          includeSettings: false,
        );
        return;
      }
      if (_applyTransferProgress(progress)) return;
    }
    switch (event.area) {
      case CoreEventArea.all:
        refreshAll();
      case CoreEventArea.presence:
        refreshAll(
          includeTransfers: false,
          includeInvitations: false,
          includeBindings: false,
          includeSettings: false,
        );
      case CoreEventArea.conversation:
        refreshAll(
          includeNearby: false,
          includeTransfers: false,
          includeInvitations: false,
          includeBindings: false,
          includeSettings: false,
        );
      case CoreEventArea.transfer:
        refreshAll(
          includeNearby: false,
          includeInvitations: false,
          includeBindings: false,
          includeSettings: false,
        );
      case CoreEventArea.invitation:
        refreshAll(
          includeNearby: false,
          includeConversations: false,
          includeTransfers: false,
          includeBindings: false,
          includeSettings: false,
        );
      case CoreEventArea.binding:
        refreshAll(
          includeNearby: false,
          includeConversations: false,
          includeTransfers: false,
          includeInvitations: false,
          includeSettings: false,
        );
    }
  }

  bool _applyTransferProgress(TransferProgressUpdate progress) {
    final index = transfers.indexWhere(
      (item) => item.id == progress.transferId,
    );
    if (index < 0) return false;
    final updated = List<TransferView>.of(transfers);
    updated[index] = updated[index].copyWith(
      state: progress.state,
      totalSize: progress.totalSize,
      persistedBytes: progress.persistedBytes,
      bytesPerSecond: progress.bytesPerSecond,
      etaSeconds: progress.etaSeconds,
      clearEtaSeconds: progress.etaSeconds == null,
      activeEntryRelativePath: progress.activeEntryRelativePath,
      clearActiveEntryRelativePath: progress.activeEntryRelativePath == null,
    );
    transfers = updated;
    notifyListeners();
    return true;
  }

  void refreshAll({
    bool includeNearby = true,
    bool includeConversations = true,
    bool includeTransfers = true,
    bool includeInvitations = true,
    bool includeBindings = true,
    bool includeSettings = true,
  }) {
    var changed = false;
    final nearbyLoader = nearbyPeerLoader;
    if (includeNearby && nearbyLoader != null) {
      try {
        nearbyDevices = nearbyLoader();
        changed = true;
      } catch (_) {
        // The next poll repairs transient core restarts and network changes.
      }
    }
    final loadConversations = conversationLoader;
    if (includeConversations && loadConversations != null) {
      try {
        final loaded = loadConversations();
        final previousSelection = selectedConversationId;
        conversations
          ..clear()
          ..addAll(loaded);
        _notifyConversationChanges(loaded);
        changed = true;
        if (loaded.isNotEmpty &&
            !loaded.any((item) => item.id == previousSelection)) {
          selectedConversationId = loaded.first.id;
        } else if (loaded.isEmpty) {
          selectedConversationId = '';
        }
        if (hasSelectedConversation) {
          _refreshSelectedMessages();
        }
      } catch (_) {
        // Keep the last database snapshot visible until the core is ready.
      }
    }
    final loadTransfers = transferLoader;
    if (includeTransfers && loadTransfers != null) {
      try {
        transfers = loadTransfers();
        _notifyTransferChanges(transfers);
        changed = true;
      } catch (_) {
        // Keep the last transfer snapshot until the core is ready.
      }
    }
    final loadInvitations = groupInvitationLoader;
    if (includeInvitations && loadInvitations != null) {
      try {
        groupInvitations = loadInvitations();
        _notifyGroupInvitations(groupInvitations);
        changed = true;
      } catch (_) {
        // Keep pending invitations visible until the next successful poll.
      }
    }
    final loadBindings = ownDeviceBindingLoader;
    if (includeBindings && loadBindings != null) {
      try {
        ownDeviceBindings = loadBindings();
        changed = true;
      } catch (_) {
        // Keep the last binding snapshot until the next successful poll.
      }
    }
    final loadSettings = settingsLoader;
    if (includeSettings && loadSettings != null) {
      try {
        settings = loadSettings();
        peerReceivePolicies = peerReceivePolicyLoader?.call() ?? const [];
        changed = true;
      } catch (_) {
        // Keep the last settings snapshot while the core is restarting.
      }
    }
    _notificationSnapshotInitialized = true;
    if (changed) notifyListeners();
  }

  void _notifyConversationChanges(List<ConversationSummary> loaded) {
    if (_notificationSnapshotInitialized &&
        settings?.notificationsEnabled != false) {
      for (final conversation in loaded) {
        final previous = _lastUnreadCounts[conversation.id] ?? 0;
        if (conversation.unreadCount > previous) {
          unawaited(
            systemNotificationCommand?.call(
                  title: '${conversation.title} 发来新消息',
                  body: conversation.preview.startsWith('[剪贴板]')
                      ? '收到新的剪贴板内容'
                      : conversation.preview,
                  conversationId: conversation.id,
                ) ??
                Future<void>.value(),
          );
        }
      }
    }
    _lastUnreadCounts
      ..clear()
      ..addEntries(loaded.map((item) => MapEntry(item.id, item.unreadCount)));
  }

  void _notifyGroupInvitations(List<GroupInvitationView> invitations) {
    if (_notificationSnapshotInitialized &&
        settings?.notificationsEnabled != false) {
      for (final invitation in invitations) {
        if (_knownInvitationIds.add(invitation.id)) {
          unawaited(
            systemNotificationCommand?.call(
                  title: '群聊邀请：${invitation.groupName}',
                  body: '打开猫猫快传查看并确认邀请',
                ) ??
                Future<void>.value(),
          );
        }
      }
    } else {
      _knownInvitationIds.addAll(invitations.map((item) => item.id));
    }
  }

  void _notifyIncomingOffers(List<TransferView> loaded) {
    final offers = loaded.where((transfer) => transfer.awaitsDecision);
    if (_notificationSnapshotInitialized &&
        settings?.notificationsEnabled != false) {
      for (final offer in offers) {
        if (_knownIncomingOfferIds.add(offer.id)) {
          unawaited(
            systemNotificationCommand?.call(
                  title: '收到文件接收请求',
                  body: offer.displayName,
                ) ??
                Future<void>.value(),
          );
        }
      }
    } else {
      _knownIncomingOfferIds.addAll(offers.map((item) => item.id));
    }
  }

  void _notifyTransferChanges(List<TransferView> loaded) {
    _notifyIncomingOffers(loaded);
    final completed = loaded.where((transfer) => transfer.state == 'completed');
    if (!_notificationSnapshotInitialized) {
      _knownCompletedTransferIds.addAll(completed.map((item) => item.id));
      return;
    }
    for (final transfer in completed) {
      if (!_knownCompletedTransferIds.add(transfer.id)) continue;
      if (settings?.notificationsEnabled != false) {
        unawaited(
          systemNotificationCommand?.call(
                title:
                    '${transfer.displayName} ${transfer.direction == 'receive' ? '已接收' : '已发送'}',
                body: '点击打开对应聊天',
                conversationId: transfer.conversationId,
              ) ??
              Future<void>.value(),
        );
      }
      if (transfer.direction != 'receive' ||
          settings?.autoOpenReceiveDirectory != true) {
        continue;
      }
      final reference = transfer.receiveBaseRef ?? transfer.localFileRef;
      if (reference != null) {
        unawaited(openReferenceCommand?.call(reference, true));
      }
    }
  }

  Future<bool> saveSettings(AppSettingsView next) async {
    final update = settingsUpdater;
    if (update == null) return false;
    try {
      lastError = null;
      settings = await update(next);
      peerReceivePolicies = peerReceivePolicyLoader?.call() ?? const [];
      notifyListeners();
      return true;
    } catch (error) {
      lastError = error.toString();
      try {
        settings = settingsLoader?.call();
      } catch (_) {}
      notifyListeners();
      return false;
    }
  }

  bool renameDevice(String deviceName) {
    final update = deviceNameUpdater;
    if (update == null) return false;
    try {
      lastError = null;
      settings = update(deviceName);
      notifyListeners();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  bool setPeerReceivePolicy(String peerDeviceId, String? policy) {
    final update = peerReceivePolicyUpdater;
    if (update == null) return false;
    try {
      lastError = null;
      update(peerDeviceId, policy);
      peerReceivePolicies = peerReceivePolicyLoader?.call() ?? const [];
      notifyListeners();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  bool requestOwnDeviceBinding(String peerDeviceId) {
    final command = requestOwnDeviceBindingCommand;
    return _runBindingCommand(
      command == null ? null : () => command(peerDeviceId),
    );
  }

  bool decideOwnDeviceBinding(String bindingId, bool accept) {
    final command = decideOwnDeviceBindingCommand;
    return _runBindingCommand(
      command == null ? null : () => command(bindingId, accept),
    );
  }

  bool removeOwnDeviceBinding(String peerDeviceId) {
    final command = removeOwnDeviceBindingCommand;
    return _runBindingCommand(
      command == null ? null : () => command(peerDeviceId),
    );
  }

  bool setClipboardMode(String peerDeviceId, String mode) {
    final command = setClipboardModeCommand;
    return _runBindingCommand(
      command == null ? null : () => command(peerDeviceId, mode),
    );
  }

  bool _runBindingCommand(void Function()? action) {
    if (action == null) return false;
    try {
      lastError = null;
      action();
      refreshAll();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  Future<String?> readClipboardText() async {
    try {
      lastError = null;
      return await clipboardReader?.call();
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return null;
    }
  }

  Future<ClipboardImageSelection?> readClipboardImage() async {
    try {
      lastError = null;
      return await clipboardImageReader?.call();
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return null;
    }
  }

  bool sendClipboardImage(ClipboardImageSelection image) {
    if (!hasSelectedConversation) return false;
    final command = sendClipboardImageCommand;
    if (command == null) return false;
    try {
      lastError = null;
      final message = command(selectedConversationId, image);
      _messagesByConversation
          .putIfAbsent(selectedConversationId, () => [])
          .add(message);
      notifyListeners();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  bool sendClipboardText(String text) {
    if (!hasSelectedConversation || text.isEmpty) return false;
    final command = sendClipboardMessageCommand;
    if (command == null) return false;
    try {
      lastError = null;
      final message = command(selectedConversationId, text);
      _messagesByConversation
          .putIfAbsent(selectedConversationId, () => [])
          .add(message);
      notifyListeners();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  int sendClipboardTextToOwnDevices(String text) {
    final command = submitClipboardTextCommand;
    if (command == null || text.isEmpty) return 0;
    var sent = 0;
    for (final binding in activeOwnDeviceBindings) {
      try {
        command(binding.peerDeviceId, text, false);
        sent++;
      } catch (error) {
        lastError = error.toString();
      }
    }
    refreshAll();
    return sent;
  }

  bool createGroup(String name, List<String> memberDeviceIds) {
    final command = createGroupCommand;
    if (command == null) return false;
    try {
      lastError = null;
      final conversation = command(name, memberDeviceIds);
      final index = conversations.indexWhere(
        (item) => item.id == conversation.id,
      );
      if (index == -1) {
        conversations.insert(0, conversation);
      } else {
        conversations[index] = conversation;
      }
      selectedConversationId = conversation.id;
      section = SidebarSection.conversations;
      mainContent = MainContent.conversation;
      compactDetailVisible = true;
      refreshAll();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  bool decideGroupInvite(String inviteId, bool accept) {
    final command = groupInviteDecisionCommand;
    if (command == null) return false;
    try {
      lastError = null;
      final groupId = command(inviteId, accept);
      refreshAll();
      if (accept &&
          groupId != null &&
          conversations.any((item) => item.id == groupId)) {
        selectConversation(groupId);
      }
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  GroupView? loadGroup(String groupId) {
    try {
      lastError = null;
      return groupLoader?.call(groupId);
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return null;
    }
  }

  bool updateGroup(
    GroupView group, {
    String? name,
    List<String> addMemberIds = const [],
    List<String> removeMemberIds = const [],
    String? transferOwnerTo,
    bool disband = false,
  }) {
    final command = groupUpdateCommand;
    if (command == null) return false;
    try {
      lastError = null;
      command(
        group.id,
        group.revision,
        name,
        addMemberIds,
        removeMemberIds,
        transferOwnerTo,
        disband,
      );
      refreshAll();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  bool leaveGroup(String groupId) {
    final command = leaveGroupCommand;
    if (command == null) return false;
    try {
      lastError = null;
      command(groupId);
      refreshAll();
      compactDetailVisible = false;
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  void selectConversation(String id) {
    selectedConversationId = id;
    _refreshSelectedMessages();
    _markSelectedConversationRead();
    section = SidebarSection.conversations;
    mainContent = MainContent.conversation;
    compactDetailVisible = true;
    notifyListeners();
  }

  void selectNearbyDevice(NearbyDevice device) {
    final open = openPrivateConversation;
    if (open != null) {
      final opened = open(device);
      final index = conversations.indexWhere((item) => item.id == opened.id);
      if (index == -1) {
        conversations.insert(0, opened);
      } else {
        conversations[index] = opened;
      }
      selectedConversationId = opened.id;
      _refreshSelectedMessages();
      _markSelectedConversationRead();
    } else {
      final exists = conversations.any(
        (conversation) => conversation.id == device.id,
      );
      if (!exists) {
        lastError = '打开会话功能未就绪';
        notifyListeners();
        return;
      }
      selectedConversationId = device.id;
    }
    section = SidebarSection.conversations;
    mainContent = MainContent.conversation;
    compactDetailVisible = true;
    notifyListeners();
  }

  bool deleteSelectedConversation() {
    if (!hasSelectedConversation || deleteConversationCommand == null) {
      return false;
    }
    final deletedId = selectedConversationId;
    try {
      lastError = null;
      deleteConversationCommand!(deletedId);
      conversations.removeWhere((item) => item.id == deletedId);
      _messagesByConversation.remove(deletedId);
      selectedConversationId = '';
      compactDetailVisible = false;
      mainContent = MainContent.conversation;
      notifyListeners();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  void showTransfers() {
    mainContent = MainContent.transfers;
    compactDetailVisible = true;
    notifyListeners();
  }

  void closeTransfers() {
    mainContent = MainContent.conversation;
    notifyListeners();
  }

  void openTransferConversation(TransferView transfer) {
    if (!conversations.any((item) => item.id == transfer.conversationId)) {
      lastError = '对应会话不存在';
      notifyListeners();
      return;
    }
    selectedConversationId = transfer.conversationId;
    focusedMessageId = transfer.messageId;
    section = SidebarSection.conversations;
    mainContent = MainContent.conversation;
    compactDetailVisible = true;
    _refreshSelectedMessages();
    _markSelectedConversationRead();
    notifyListeners();
  }

  void clearFocusedMessage(String messageId) {
    if (focusedMessageId == messageId) focusedMessageId = null;
  }

  int clearCompletedTransfers() {
    final command = clearCompletedTransfersCommand;
    if (command == null) return 0;
    try {
      lastError = null;
      final count = command();
      refreshAll(
        includeNearby: false,
        includeConversations: false,
        includeInvitations: false,
        includeBindings: false,
        includeSettings: false,
      );
      return count;
    } catch (error) {
      lastError = errorPresenter(error);
      notifyListeners();
      return -1;
    }
  }

  void showCompactList() {
    compactDetailVisible = false;
    notifyListeners();
  }

  bool sendText(String text) {
    if (text.trim().isEmpty || !hasSelectedConversation) return false;
    final command = sendTextCommand;
    if (command == null) return false;
    try {
      lastError = null;
      final message = command(selectedConversationId, text);
      _messagesByConversation
          .putIfAbsent(selectedConversationId, () => [])
          .add(message);
      notifyListeners();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  Future<bool> sendSource(MessageVisualKind kind) async {
    if (!hasSelectedConversation) return false;
    final wireKind = switch (kind) {
      MessageVisualKind.file => 'file',
      MessageVisualKind.image => 'image',
      MessageVisualKind.folder => 'folder',
      _ => throw ArgumentError.value(kind, 'kind', 'unsupported source kind'),
    };
    try {
      lastError = null;
      final pickAndSend = pickAndSendSourceCommand;
      if (pickAndSend != null) {
        if (!await pickAndSend(selectedConversationId, wireKind)) return false;
        refreshAll();
        return true;
      }
      final pick = sourcePicker;
      final send = sendSourceCommand;
      if (pick == null || send == null) return false;
      final sourcePath = await pick(wireKind);
      if (sourcePath == null) return false;
      send(selectedConversationId, sourcePath, wireKind);
      refreshAll();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  Future<bool> acceptIncomingOffer(String transferId) async {
    try {
      lastError = null;
      final accept = acceptIncomingOfferCommand;
      if (accept != null) {
        if (!await accept(transferId)) return false;
        refreshAll();
        return true;
      }
      final pick = sourcePicker;
      final decide = decideIncomingOfferCommand;
      if (pick == null || decide == null) return false;
      final receiveDirectory = await pick('folder');
      if (receiveDirectory == null) return false;
      decide(transferId, true, receiveDirectory);
      refreshAll();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  bool rejectIncomingOffer(String transferId) {
    final decide = decideIncomingOfferCommand;
    if (decide == null) return false;
    try {
      lastError = null;
      decide(transferId, false, null);
      refreshAll();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  bool pauseTransfer(String transferId) =>
      _runTransferCommand(pauseTransferCommand, transferId);

  bool resumeTransfer(String transferId) =>
      _runTransferCommand(resumeTransferCommand, transferId);

  bool cancelTransfer(String transferId) =>
      _runTransferCommand(cancelTransferCommand, transferId);

  Future<bool> reselectTransferSource(TransferView transfer) async {
    final command = reselectTransferSourceCommand;
    if (command == null) return false;
    try {
      lastError = null;
      final replaced = await command(transfer);
      if (replaced) refreshAll();
      return replaced;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  bool _runTransferCommand(TransferCommand? command, String transferId) {
    if (command == null) return false;
    try {
      lastError = null;
      command(transferId);
      refreshAll();
      return true;
    } catch (error) {
      lastError = error.toString();
      notifyListeners();
      return false;
    }
  }

  void _refreshSelectedMessages() {
    final loader = messageLoader;
    if (loader == null || !hasSelectedConversation) return;
    try {
      _messagesByConversation[selectedConversationId] = List<ChatMessage>.of(
        loader(selectedConversationId),
      );
    } catch (_) {
      // Keep the current message list until the next poll succeeds.
    }
  }

  void _markSelectedConversationRead() {
    final markRead = markConversationReadCommand;
    if (markRead == null || !hasSelectedConversation) return;
    final index = conversations.indexWhere(
      (item) => item.id == selectedConversationId,
    );
    if (index < 0 || conversations[index].unreadCount == 0) return;
    final messages =
        _messagesByConversation[selectedConversationId] ?? const [];
    final throughSortOrder = messages.fold<int>(
      0,
      (maximum, message) =>
          message.localSortOrder > maximum ? message.localSortOrder : maximum,
    );
    try {
      markRead(selectedConversationId, throughSortOrder);
      conversations[index] = conversations[index].copyWith(unreadCount: 0);
    } catch (error) {
      lastError = error.toString();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _nearbyTimer?.cancel();
    _eventReconnectTimer?.cancel();
    _coreEventSubscription?.cancel();
    super.dispose();
  }
}
