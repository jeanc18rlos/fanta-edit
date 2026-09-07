Feature: Visual Design Canvas
  As a user of Fanta Edit
  I want to open and visually edit design documents (.fig or Fanta projects)
  So that I can design UIs that stay in sync with source code

  Background:
    Given Fanta Edit is running
    And a project is open

  Scenario: Open a .fig file and render the canvas
    Given I have a .fig design file in the project
    When I open the design file
    Then the visual canvas renders the document
    And the active page is displayed
    And the design layers panel is visible
    And the properties inspector is visible

  Scenario: Create basic shapes on the canvas
    Given a design document is open on the canvas
    When I select the rectangle tool
    And I drag to create a rectangle at position (100, 100) size (200, 100)
    Then a rectangle node appears in the scene
    And it is selected
    And its properties are visible in the inspector

  Scenario: Use frames and nesting
    Given a design document is open
    When I create a frame
    And I create a rectangle inside the frame
    Then the rectangle is parented under the frame
    And world coordinates are preserved correctly after nesting

  Scenario: Edit properties via inspector
    Given a rectangle node is selected
    When I change its fill color in the inspector
    And I change its corner radius
    Then the canvas updates the visual appearance immediately
    And the change is part of the undo stack

  Scenario: Text editing on canvas
    Given a text tool is active
    When I create a text node with initial content "Hello"
    Then the text node renders with the default font
    And I can edit the text directly or via inspector

  Scenario: Multiple selection and reparenting
    Given several nodes are selected
    When I reparent them under a new container
    Then their world positions are maintained
    And z-order follows creation order within the parent

  Scenario: Delete selection
    Given nodes are selected on the canvas
    When I press delete
    Then the nodes and their children are removed
    And the change can be undone

  Scenario: Zoom, pan, and fit view
    Given a large design is open
    When I zoom in and pan around
    And I trigger "Fit to View"
    Then the viewport centers and scales the content appropriately
    And viewport state is persisted

  Scenario: Undo and redo across visual operations
    Given I have performed several canvas edits
    When I undo
    Then the previous state is restored on the canvas and panels
    When I redo
    Then the forward state is restored

  Scenario: Save design document as Fanta project
    Given an unsaved .fig or in-memory design
    When I save the document
    Then a Fanta project structure is created with pages/ and assets/
    And .fnx source files are materialized for the pages
    And subsequent opens load the project with full fidelity

  Scenario: Error on canvas shows helpful message
    Given a design that fails to load
    When the canvas attempts to render it
    Then a clear error screen is shown with the full error chain

Feature: FNX Source + Visual Synchronization
  As a developer/designer
  I want edits in the visual canvas to produce reviewable source
  And code edits in .fnx to update the canvas

  Scenario: Visual edit materializes and updates FNX
    Given a Fanta project is open with a page
    When I create nodes via the canvas
    And I save
    Then .fnx files exist under pages/<id>/
    And the FNX contains the created nodes in JSX-like form

  Scenario: Editing FNX on disk reloads the canvas
    Given a Fanta project page is open on canvas
    When I edit the corresponding .fnx file with valid changes (colors wrapped in fnxColor, proper structure)
    And I save the FNX buffer
    Then the canvas reloads and reflects the FNX changes after debounce
    And no "source_edit_locked" error occurs when canvas is clean

  Scenario: Mixed editing is protected
    Given unsaved edits exist in an FNX buffer for a page
    When I attempt a design_edit operation on the canvas
    Then the operation is rejected with source_edit_locked
    And I must save the FNX buffer first

  Scenario: Advanced styling only possible in FNX
    Given a node on canvas
    When I want gradients, shadows, auto-layout or variable bindings
    Then I edit the .fnx source
    And save to see updates on canvas

Feature: Motion, Timeline and Prototypes
  As a designer
  I want to author animations and interactive prototypes

  Scenario: Create and edit motion clips
    Given a design with nodes
    When I open the motion panel and create a clip with tracks
    And I add keyframes for transform properties
    Then the timeline shows the tracks and keyframes
    And scrubbing previews the animation

  Scenario: Prototype playback
    Given frames configured as prototype starting points and destinations
    When I play the prototype
    Then interactions (clicks/taps) navigate between prototype frames
    And I can exit prototype mode

  Scenario: Easing and interpolation editing
    Given a keyframe selection in the timeline
    When I change easing or interpolation
    Then the motion evaluation updates
    And invalid changes are rejected gracefully

Feature: Agent-driven Design Operations (DesignSurface)
  As an AI agent user
  I want the agent to inspect and edit the design canvas safely using structured tools

  Scenario: Agent reads design state
    Given a design document is focused
    When the agent calls design_state
    Then it receives pages, selection, viewport, and node tree
    And ids are stable

  Scenario: Agent applies batch design edits transactionally
    Given a design is open
    When the agent calls design_edit with a batch of create_node, set_props, reparent
    And one op would fail
    Then the entire batch rolls back
    And the result reports which op failed and why
    And a single undo step is created with the provided label

  Scenario: Agent requests screenshots for verification
    Given nodes on a page
    When the agent calls design_screenshot for a frame or node
    Then a PNG is returned
    And it can be used to verify layout

  Scenario: Agent places generated images
    Given media generation is available
    When the agent uses place_generation or create_image
    Then the image is downloaded/ingested as an asset
    And placed on the canvas with provenance recorded

Feature: Code Workspace alongside Visual
  As a user working on design systems
  I want to see and edit the generated source while designing

  Scenario: Fanta code workspace shows derived sources
    Given a Fanta project is open in the visual canvas
    When I open the associated code workspace view
    Then source files (FNX, possibly component masters) are accessible
    And edits there affect the visual document on save

Feature: Panels, Settings and UI Polish
  Scenario: Toggle design and properties panels
    Given a canvas is open
    When I toggle panel visibility via settings or UI
    Then panels show/hide and sizes are remembered

  Scenario: Variables and component instances
    Given a project using variables and component instances
    When I inspect and edit bindings in the variables workspace or inspector
    Then values propagate and canvas updates

  Scenario: Comments on design
    Given a design document
    When users or agents add and resolve comments
    Then comments persist and are visible in the comments panel
